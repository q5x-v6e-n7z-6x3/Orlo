//! The custom element that lays out and paints the visible display rows.

use super::*;
use crate::config::settings::WhiteSpace;
use crate::editing::display::filler::Padded;
use crate::editing::display::{expand_tabs_at, top_anchor, top_row, wrap_width};
use crate::editing::fold::{Folded, Piece, PLACEHOLDER, PLACEHOLDER_COLS};
use crate::editing::text::{DisplayRow, Document, Rows, Selection};
use crate::languages::highlight::{Look, Run};
use gpui::{
    fill, linear_color_stop, linear_gradient, outline, point, px, quad, relative, size, App, BorderStyle, Bounds,
    ContentMask, Corners, DispatchPhase, Edges, Element, ElementId, ElementInputHandler, Entity, FontStyle, FontWeight,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent,
    PaintQuad, Path, Pixels, ScrollWheelEvent, ShapedLine, SharedString, Style, TextRun, UnderlineStyle, Window,
};
use std::borrow::Cow;
use std::time::{Duration, Instant};

pub(crate) struct Layout {
    pub(crate) bounds: Bounds<Pixels>,
    /// The scroll this frame was drawn at (mid-glide, not where Page Down is going): clicks and
    /// the IME go by what is on screen.
    pub(crate) scroll_top: f64,
    pub(crate) text_left: Pixels,
    pub(crate) text_width: Pixels,
    /// One space's width: a continuation row is drawn its indent times this right of `text_left`.
    pub(crate) space_w: Pixels,
    /// The font and size rows are shaped in, for measuring rows this frame did not draw.
    pub(crate) font: gpui::Font,
    pub(crate) font_size: Pixels,
    pub(crate) first_row: usize,
    /// The drawn rows from `first_row`: the column each starts at, and its text as shaped;
    /// `None` for a filler row, which has no text. They can be a frame older than the text, so
    /// offsets come from the state, not from here.
    pub(crate) rows: Vec<Option<(usize, ShapedLine)>>,
    /// Left edge and first drawn row of the minimap.
    pub(crate) minimap: Option<(Pixels, usize)>,
    /// Between the gutter and the minimap: where dragged text may drop.
    pub(crate) text_area: Bounds<Pixels>,
    #[cfg(test)]
    pub(crate) drawn: Drawn,
}

/// What a frame drew, for tests: carets, fold buttons as (line, folded), markers of folded
/// text by line, minimap rows, white space marks.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Drawn {
    pub(crate) carets: Vec<Bounds<Pixels>>,
    /// Cursor-column background quads supplied to paint, by text display row.
    pub(crate) cursor_columns: Vec<(usize, Bounds<Pixels>, Hsla)>,
    pub(crate) cursor_column_shapes: usize,
    pub(crate) cursor_column_chars: usize,
    pub(crate) fold_buttons: Vec<(usize, bool)>,
    pub(crate) markers: Vec<(usize, Bounds<Pixels>)>,
    pub(crate) minimap_rows: usize,
    /// The white space marks: the char offset, the mark, the cell it is in (a tab's runs to
    /// its next tab stop, a line end's is a space wide, after the text) and its colour.
    pub(crate) marks: Vec<(usize, Mark, Bounds<Pixels>, Hsla)>,
    /// The thin lines that run on from the arrows of tabs to their next stops, as painted.
    pub(crate) tab_lines: Vec<PaintQuad>,
    pub(crate) unicode_warnings: Vec<(usize, char, Bounds<Pixels>)>,
    pub(crate) unicode_visits: usize,
    pub(crate) unicode_warning_colors: Vec<Hsla>,
    pub(crate) spelling: Vec<Range<usize>>,
    pub(crate) occurrences: Vec<(Range<usize>, Bounds<Pixels>)>,
    pub(crate) inline_diagnostics: Vec<(usize, usize, String, Bounds<Pixels>)>,
    pub(crate) occurrence_visits: usize,
    /// The filler rows drawn, by display row.
    pub(crate) fillers: Vec<usize>,
    /// (display row, line number) for each number drawn in the gutter.
    pub(crate) numbers: Vec<(usize, usize)>,
    /// The actual shaped gutter text, by display row.
    pub(crate) number_labels: Vec<(usize, SharedString)>,
    /// The display rows a selection's background was drawn on.
    pub(crate) selected: Vec<usize>,
    pub(crate) selected_bounds: Vec<(usize, Bounds<Pixels>)>,
    /// The display rows whose gutter shows the current line's highlight.
    pub(crate) current: Vec<usize>,
    pub(crate) comparison: Vec<(Range<usize>, Bounds<Pixels>, Hsla, bool)>,
    pub(crate) comparison_visits: usize,
}

#[cfg(test)]
thread_local! {
    /// Bookmark dots drawn on this thread, for a test to see one drawn.
    pub static BOOKMARK_DOTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub struct EditorElement {
    pub(crate) editor: Entity<Editor>,
}

fn spell_underlines(
    mut runs: Vec<TextRun>,
    rope: &ropey::Rope,
    text: &RowText,
    row: Range<usize>,
    ranges: impl IntoIterator<Item = Range<usize>>,
) -> Vec<TextRun> {
    for range in ranges {
        let (start, end) = (range.start.max(row.start), range.end.min(row.end));
        if start < end {
            let (a, b) = (text.byte_of(rope, start), text.byte_of(rope, end));
            runs = restyle(runs, a, b, |r| r.underline = underline(c(0xec5f66), true));
        }
    }
    runs
}

/// A 1 pt underline, straight or wavy.
fn underline(color: Hsla, wavy: bool) -> Option<UnderlineStyle> {
    Some(UnderlineStyle { thickness: px(1.), color: Some(color), wavy })
}

/// A run of `len` bytes in `font` and `color`, unstyled.
pub(super) fn text_run(len: usize, font: &gpui::Font, color: Hsla) -> TextRun {
    TextRun { len, font: font.clone(), color, background_color: None, underline: None, strikethrough: None }
}

/// Shape at most the prepared bounded preview, eliding only at glyph boundaries.
fn elide_inline(text: &SharedString, width: Pixels, shape: impl Fn(SharedString) -> ShapedLine) -> Option<ShapedLine> {
    if text.trim().is_empty() || width <= px(0.) {
        return None;
    }
    let shaped = shape(text.clone());
    if shaped.width <= width {
        return Some(shaped);
    }
    let ellipsis = shape("…".into());
    if width < ellipsis.width {
        return None;
    }
    let mut end = shaped.index_for_x(width - ellipsis.width).unwrap_or(text.len());
    loop {
        let elided = shape(format!("{}…", &text[..end]).into());
        if elided.width <= width {
            return Some(elided);
        }
        end = text[..end].char_indices().next_back()?.0;
    }
}

/// A filled box with round corners.
fn rounded(bounds: Bounds<Pixels>, radius: Pixels, color: Hsla) -> PaintQuad {
    quad(bounds, Corners::all(radius), color, Edges::default(), color, BorderStyle::Solid)
}

/// A left press on the view, before what is drawn under the pointer hears it.
fn left_press(e: &MouseDownEvent, phase: DispatchPhase, hitbox: &Hitbox, window: &Window) -> bool {
    phase == DispatchPhase::Capture && e.button == MouseButton::Left && hitbox.is_hovered(window)
}

pub struct Prepaint {
    /// The view under the pointer, unless something drawn over it takes the mouse (the
    /// palette, a menu, a divider): gutter and marker clicks and hover go by it.
    hitbox: Hitbox,
    /// (y, x, shaped text, shaped line number)
    lines: Vec<(Pixels, Pixels, ShapedLine, Option<ShapedLine>)>,
    gutter_right: Pixels,
    /// Text, selections and carets are clipped to this so sideways scrolling never
    /// paints over the gutter or minimap.
    text_mask: Bounds<Pixels>,
    gutter: Vec<PaintQuad>,
    change_hits: Vec<(Bounds<Pixels>, usize, crate::integrations::changes::Hunk)>,
    changes_version: u64,
    /// The fold buttons drawn in the gutter: where (a row's 20 pt beside the text), whether folded, colour.
    fold_buttons: Vec<(Bounds<Pixels>, bool, Hsla)>,
    /// Where a click toggles a line's fold (its button, drawn or faded) or opens a fold (its
    /// placeholder's marker).
    fold_hits: Vec<(Bounds<Pixels>, FoldHit)>,
    cursor_columns: Vec<PaintQuad>,
    below: Vec<PaintQuad>,
    carets: Vec<PaintQuad>,
    /// The find outlines, the markers of folded text and the lines of tabs: over selections,
    /// under carets.
    outlines: Vec<PaintQuad>,
    /// The white space marks: each one's glyph and the top left it is painted at.
    marks: Vec<(ShapedLine, Point<Pixels>)>,
    inline_diagnostics: Vec<(ShapedLine, Point<Pixels>)>,
    minimap: Vec<PaintQuad>,
    /// Where the minimap and the gutter are, for what hovering them shows.
    minimap_bounds: Option<Bounds<Pixels>>,
    gutter_bounds: Bounds<Pixels>,
    scrollbar: Option<PaintQuad>,
    /// Sublime's shadow at the text's right edge while lines run on past it.
    shadow: Option<PaintQuad>,
}

#[derive(Debug, PartialEq, Eq)]
enum ChangeMarker {
    Added,
    Modified,
    /// A deletion at the start of this line, or at the end of the final line.
    Deleted(bool),
}

fn change_marker(
    hunks: &[crate::integrations::changes::Hunk],
    line: usize,
    lines: usize,
) -> Option<(usize, ChangeMarker)> {
    hunks.iter().enumerate().find_map(|(index, hunk)| {
        if hunk.new_lines.contains(&line) {
            Some((index, if hunk.old_lines.is_empty() { ChangeMarker::Added } else { ChangeMarker::Modified }))
        } else if hunk.new_lines.is_empty() && hunk.new_lines.start.min(lines - 1) == line {
            Some((index, ChangeMarker::Deleted(hunk.new_lines.start >= lines)))
        } else {
            None
        }
    })
}

/// Indent levels to draw guides for; a blank line continues the deeper of its neighbours.
fn guide_levels(doc: &Document, line: usize) -> usize {
    // the leading whitespace in columns, read off the rope: a 232k-char line costs its indent
    let width = |l: usize| crate::editing::fold::indent(doc.rope.line(l), doc.tab_size);
    let w = width(line).unwrap_or_else(|| {
        // looks at most 100 lines each way for a non-blank neighbour
        let prev = (line.saturating_sub(100)..line).rev().find_map(width).unwrap_or(0);
        let next = (line + 1..(line + 100).min(doc.line_count())).find_map(width).unwrap_or(0);
        prev.max(next)
    });
    w / doc.tab_size
}

/// What Show White Space draws for a kind of white space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Space,
    Tab,
    /// A non-breaking, en, em or ideographic space.
    Other,
    /// The line break after a line.
    Eol,
}

impl Mark {
    fn glyph(self) -> &'static str {
        match self {
            Mark::Space => "·",
            Mark::Tab => "→",
            Mark::Other => "°",
            Mark::Eol => "¬",
        }
    }
}

/// The mark for a char, if it is white space: a space, a tab or one of the Unicode spaces.
fn white(c: char) -> Option<Mark> {
    match c {
        ' ' => Some(Mark::Space),
        '\t' => Some(Mark::Tab),
        '\u{a0}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' => Some(Mark::Other),
        _ => None,
    }
}

/// The colours of white space marks: the indent guides' on plain text, and on selected text the
/// guide colour halfway to the foreground, as the guide colour is lost against a selection
/// (1.1:1 on Mariana's).
fn mark_colors(guide: Hsla, foreground: Hsla) -> (Hsla, Hsla) {
    let (g, f) = (guide.to_rgb(), foreground.to_rgb());
    let half = |a: f32, b: f32| (a + b) / 2.;
    let mixed = gpui::Rgba { r: half(g.r, f.r), g: half(g.g, f.g), b: half(g.b, f.b), a: half(g.a, f.a) };
    (guide, mixed.into())
}

/// Warning ink is independent of theme keys (the default amber is also the caret key).
/// Keep amber where it reads; otherwise black/white guarantees 4.5:1 on an opaque surface.
fn warning_color(surface: Hsla) -> Hsla {
    let luminance = |color: Hsla| {
        let rgb = color.to_rgb();
        let linear = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
    };
    let amber = gpui::rgb(0xf9ae58).into();
    let (bg, ink) = (luminance(surface), luminance(amber));
    if (bg.max(ink) + 0.05) / (bg.min(ink) + 0.05) >= 4.5 {
        amber
    } else if (bg + 0.05) / 0.05 >= 1.05 / (bg + 0.05) {
        gpui::black()
    } else {
        gpui::white()
    }
}

/// The non-empty selection of `sels` (sorted, apart) that takes in the char at `at`.
fn selected_at(sels: &[Selection], at: usize) -> Option<&Selection> {
    let i = sels.partition_point(|s| s.end() <= at);
    sels.get(i).filter(|s| s.start() <= at)
}

/// The marks `ws` puts on the chars `seg` of `line` (a whole line, or the part of one from a
/// row on), as (char, mark) in order: white space in the selections `sels` (sorted, apart, as
/// char ranges of `line`), white space after the last char that is not, or all of it; with
/// `all`, `Eol` after the last char when `seg` ends the line and a line break follows it
/// (`has_eol`).
pub fn marks(
    line: &str,
    seg: Range<usize>,
    sels: &[Range<usize>],
    ws: WhiteSpace,
    has_eol: bool,
) -> Vec<(usize, Mark)> {
    let n = line.chars().count();
    // trailing white space starts here; past the end when it is not asked for
    let trail = match ws.trailing {
        true => n - line.chars().rev().take_while(|&c| white(c).is_some()).count(),
        false => n,
    };
    let selected = |at: usize| {
        let i = sels.partition_point(|r| r.end <= at);
        ws.selection && sels.get(i).is_some_and(|r| r.start <= at)
    };
    let chars = line.chars().enumerate().skip(seg.start).take(seg.len());
    let mut out: Vec<_> = chars
        .filter_map(|(at, c)| {
            let mark = white(c)?;
            (ws.all || at >= trail || selected(at)).then_some((at, mark))
        })
        .collect();
    if ws.all && has_eol && seg.end == n {
        out.push((n, Mark::Eol));
    }
    out
}

/// The non-empty selections that touch the chars `a..b`, as ranges counted from `a`.
fn selected_in(sels: &[Selection], a: usize, b: usize) -> Vec<Range<usize>> {
    // selections are sorted and apart: the first one ending after `a` on
    let from = sels.partition_point(|s| s.end() <= a);
    (sels[from..].iter())
        .take_while(|s| s.start() < b)
        .filter(|s| !s.is_empty())
        .map(|s| s.start().saturating_sub(a)..s.end() - a)
        .collect()
}

/// The x of each char of a shaped row, and past its last, as `x_for_index` gives it for the
/// bytes of `shown` at `cols`: a pass over the glyphs, not a scan to each char.
fn char_xs(shown: &str, cols: &[usize], shaped: &ShapedLine) -> Vec<Pixels> {
    let bytes: Vec<usize> = (shown.char_indices()).map(|(b, _)| b).chain([shown.len()]).collect();
    let mut glyphs = shaped.runs.iter().flat_map(|r| &r.glyphs).peekable();
    (cols.iter())
        .map(|&col| {
            while glyphs.next_if(|g| g.index < bytes[col]).is_some() {}
            glyphs.peek().map_or(shaped.width, |g| g.position.x)
        })
        .collect()
}

/// Run lengths, in bytes of `text`, re-measured in its tab-expanded form.
fn expand_runs(text: &str, cols: &[usize], runs: &mut [TextRun]) {
    let (mut byte, mut ch) = (0, 0);
    for r in runs {
        let n = text[byte..byte + r.len].chars().count();
        byte += r.len;
        r.len = r.len - n + cols[ch + n] - cols[ch];
        ch += n;
    }
}

/// Mariana's italic, bold, underline and background on a run.
fn styled(mut r: TextRun, look: Look) -> TextRun {
    if look.italic() {
        r.font.style = FontStyle::Italic;
    }
    if look.bold() {
        r.font.weight = FontWeight::BOLD;
    }
    if look.underline() {
        r.underline = underline(r.color, false);
    }
    r.background_color = look.background;
    r
}

/// `runs` (lengths in bytes of their text) with the bytes `a..b` in `color`: `recolor_each`
/// for one range, which the tests check it against.
#[cfg(test)]
fn recolor(runs: Vec<TextRun>, a: usize, b: usize, color: Hsla) -> Vec<TextRun> {
    restyle(runs, a, b, |r| r.color = color)
}

/// `recolor` for each of `ranges` (sorted, disjoint byte ranges and their colours) in one pass
/// over the runs: a row of a thousand Find All matches is not re-split a thousand times.
fn recolor_each(runs: Vec<TextRun>, ranges: &[(usize, usize, Hsla)]) -> Vec<TextRun> {
    let mut out = Vec::with_capacity(runs.len() + 2 * ranges.len());
    let (mut at, mut k) = (0, 0);
    for r in runs {
        let (s, e) = (at, at + r.len);
        at = e;
        let mut x = s;
        while x < e {
            while ranges.get(k).is_some_and(|&(a, b, _)| b <= x || a >= b) {
                k += 1;
            }
            let (to, color) = match ranges.get(k) {
                Some(&(a, b, color)) if a <= x => (b.min(e), Some(color)),
                Some(&(a, _, _)) => (a.min(e), None),
                None => (e, None),
            };
            let mut piece = r.clone();
            piece.len = to - x;
            if let Some(color) = color {
                piece.color = color;
            }
            out.push(piece);
            x = to;
        }
    }
    out
}

/// `runs` with the bytes `a..b` split off and changed by `f`.
fn restyle(runs: Vec<TextRun>, a: usize, b: usize, f: impl Fn(&mut TextRun)) -> Vec<TextRun> {
    let mut out = Vec::with_capacity(runs.len() + 2);
    let mut at = 0;
    for r in runs {
        let (s, e) = (at, at + r.len);
        at = e;
        let (ca, cb) = (a.clamp(s, e), b.clamp(s, e));
        if ca == cb && (ca == s || ca == e) {
            if e > s {
                out.push(r);
            }
            continue;
        }
        for (x, y, changed) in [(s, ca, false), (ca, cb, true), (cb, e, false)] {
            if y > x {
                let mut piece = r.clone();
                piece.len = y - x;
                if changed {
                    f(&mut piece);
                }
                out.push(piece);
            }
        }
    }
    out
}

/// The chars of the row `ls..le` a diagnostic over `d` underlines, if it is on the row (at
/// least one char): one that ends where the row starts is the row before's.
fn diagnostic_on_row(d: &Range<usize>, ls: usize, le: usize) -> Option<Range<usize>> {
    (d.start <= le && (d.end > ls || d.start == ls)).then(|| {
        let start = d.start.min(le.saturating_sub(1)).max(ls);
        start..d.end.max(start + 1).min(le)
    })
}

/// One row's piece of a selection: x from, x to, y, and whether the selection starts or ends
/// on this row.
type SelRect = (f32, f32, f32, bool, bool);

/// Sublime's 1pt selection border around one selection's rows: down both sides of each row,
/// and across wherever a row sticks out past the one above or below. Filler rows between two
/// of them are not selected: the border closes above them and opens again below.
fn selection_border(rects: &[SelRect], lh: f32, color: Hsla, out: &mut Vec<PaintQuad>) {
    let mut line = |x0: f32, x1: f32, y: f32| {
        if x1 > x0 {
            out.push(fill(Bounds::from_corners(point(px(x0), px(y)), point(px(x1), px(y + 1.))), color));
        }
    };
    for (i, &(x0, x1, y, first, last)) in rects.iter().enumerate() {
        match i.checked_sub(1).map(|j| rects[j]) {
            None if first => line(x0, x1, y),
            None => {}
            Some((a0, a1, ay, ..)) if ay + lh < y => {
                line(a0, a1, ay + lh - 1.);
                line(x0, x1, y);
            }
            Some((a0, a1, ..)) if x1 <= a0 || x0 >= a1 => {
                line(a0, a1, y - 1.);
                line(x0, x1, y);
            }
            Some((a0, a1, ..)) => {
                line(a0.min(x0), a0.max(x0), y);
                line(a1.min(x1), a1.max(x1), y);
            }
        }
        if last {
            line(x0, x1, y + lh - 1.);
        }
    }
    for &(x0, x1, y, ..) in rects {
        out.push(fill(Bounds::from_corners(point(px(x0), px(y)), point(px(x0 + 1.), px(y + lh))), color));
        out.push(fill(Bounds::from_corners(point(px(x1 - 1.), px(y)), point(px(x1), px(y + lh))), color));
    }
}

/// The coloured runs of a line (bytes) cut to the bytes `b0..b1` of one of its rows.
fn slice_runs(colored: &[Run], b0: usize, b1: usize) -> Vec<Run> {
    let mut at = 0;
    let mut out = Vec::new();
    for &(len, color, look) in colored {
        let (s, e) = (at.max(b0), (at + len).min(b1));
        if s < e {
            out.push((e - s, color, look));
        }
        at += len;
    }
    out
}

/// What painting needs of one display row: the first buffer line of its display line (its
/// number, gutter marks and guides), its chars as buffer offsets (hidden ones included, without
/// the line break), the display column it starts at (a continuation row's indent), and its text
/// and its folds' placeholders in order.
pub(super) type Row = orlo_editor::rendering::folded::Segment;

/// Shown row `r` of `rows`, the view's padded rows of display lines; `None` for a filler row.
pub(super) fn row(doc: &Document, rows: &Padded<Folded>, r: usize) -> Option<Row> {
    let r = rows.inner_row(doc, DisplayRow(r))?;
    Some(rows.inner().segment(doc, r))
}

/// The buffer line of the text row nearest display row `r` (down first, then up): a range of
/// rows that are all filler rows still has lines around it.
fn line_near(doc: &Document, rows: &Padded<Folded>, r: usize) -> usize {
    let r = DisplayRow(r.min(rows.rows(doc).saturating_sub(1)));
    let text = rows.text_row(doc, r, true).or_else(|| rows.text_row(doc, r, false));
    text.and_then(|t| row(doc, rows, t.0)).map_or(0, |row| row.line)
}

/// Line `line`'s coloured runs, for `RowText::runs` to keep for the rows after it. A line past
/// what this frame may parse widens `unready`, and draws in its colours from before an edit
/// while it waits, if its text is the same.
fn line_colors(
    highlighter: &mut crate::languages::highlight::Highlighter,
    rope: &ropey::Rope,
    line: usize,
    single: bool,
    unready: &mut Option<(usize, usize)>,
) -> Vec<Run> {
    if single {
        Vec::new()
    } else if let Some(runs) = highlighter.try_line_runs(rope, line) {
        runs
    } else {
        *unready = Some(unready.map_or((line, line), |(a, b)| (a.min(line), b.max(line))));
        highlighter.fallback_runs(rope, line).unwrap_or_default()
    }
}

/// What a click in the gutter or on a marker does: fold or unfold the region a line heads, or
/// open the fold whose placeholder it is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum FoldHit {
    Toggle(usize),
    Open(usize),
}

/// A row's text as it is shaped, before tabs are expanded: its pieces' chars, each placeholder
/// `PLACEHOLDER_COLS` spaces (the marker is drawn over them), and where each piece starts in it.
pub(super) struct RowText {
    pub(super) text: String,
    /// Per piece: its buffer chars, the char and the byte of `text` it starts at, and whether
    /// it is a fold.
    pieces: Vec<(Range<usize>, usize, usize, bool)>,
}

impl RowText {
    pub(super) fn new(rope: &ropey::Rope, pieces: &[Piece]) -> Self {
        let mut out = Self { text: String::new(), pieces: Vec::with_capacity(pieces.len()) };
        let mut chars = 0;
        for p in pieces {
            let (Piece::Text(r) | Piece::Fold(r)) = p;
            let fold = matches!(p, Piece::Fold(_));
            out.pieces.push((r.clone(), chars, out.text.len(), fold));
            match fold {
                false => out.text.extend(rope.slice(r.clone()).chunks()),
                true => out.text.extend(std::iter::repeat_n(' ', PLACEHOLDER_COLS)),
            }
            chars += piece_width(p);
        }
        out
    }

    /// Syntax runs shared by the editor and minimap, including fold placeholders and plain
    /// text beyond the highlighter's current coverage. A line's colours come from `colors` and
    /// are kept in `cached` for the rows after it, lent rather than cloned per row.
    fn runs(
        &self,
        rope: &ropey::Rope,
        color: Hsla,
        run: impl Fn(usize, Hsla) -> TextRun,
        cached: &mut Option<(usize, Vec<Run>)>,
        mut colors: impl FnMut(usize) -> Vec<Run>,
    ) -> Vec<TextRun> {
        let mut runs: Vec<TextRun> = Vec::new();
        for (r, _, _, fold) in &self.pieces {
            if *fold {
                runs.push(run(PLACEHOLDER_COLS, color));
                continue;
            }
            let l = rope.char_to_line(r.start);
            if cached.as_ref().map(|c| c.0) != Some(l) {
                *cached = Some((l, colors(l)));
            }
            let colored = cached.as_ref().map_or(&[][..], |c| &c.1);
            // Use rope byte offsets so long lines do not need scanning per row.
            let lb = rope.char_to_byte(rope.line_to_char(l));
            let (b0, b1) = (rope.char_to_byte(r.start) - lb, rope.char_to_byte(r.end) - lb);
            let at = runs.len();
            runs.extend((slice_runs(colored, b0, b1).into_iter()).map(|(n, col, look)| styled(run(n, col), look)));
            let covered: usize = runs[at..].iter().map(|r| r.len).sum();
            if covered < b1 - b0 {
                runs.push(run(b1 - b0 - covered, color));
            }
        }
        runs
    }

    /// The piece offset `o` is in or after, if any starts at or before it.
    fn piece(&self, o: usize) -> Option<&(Range<usize>, usize, usize, bool)> {
        let i = self.pieces.partition_point(|p| p.0.start <= o);
        self.pieces.get(i.checked_sub(1)?)
    }

    /// The char of `text` offset `o` is drawn before: hidden text at its fold's start, a fold's
    /// end past its placeholder; offsets outside the row at its ends.
    fn char_of(&self, o: usize) -> usize {
        match self.piece(o) {
            None => 0,
            Some((r, c, _, false)) => c + o.min(r.end) - r.start,
            Some((r, c, _, true)) if o < r.end => *c,
            Some((_, c, _, true)) => c + PLACEHOLDER_COLS,
        }
    }

    /// `char_of` in bytes of `text`.
    fn byte_of(&self, rope: &ropey::Rope, o: usize) -> usize {
        match self.piece(o) {
            None => 0,
            Some((r, _, b, false)) => b + rope.char_to_byte(o.min(r.end)) - rope.char_to_byte(r.start),
            Some((r, _, b, true)) if o < r.end => *b,
            Some((_, _, b, true)) => b + PLACEHOLDER_COLS,
        }
    }

    /// The offset char `c` of `text` shows; a placeholder's chars show its fold's start.
    fn offset_of(&self, c: usize) -> usize {
        let i = self.pieces.partition_point(|p| p.1 <= c);
        match i.checked_sub(1).map(|i| &self.pieces[i]) {
            None => 0,
            Some((r, at, _, false)) => (r.start + c - at).min(r.end),
            Some((r, _, _, true)) => r.start,
        }
    }

    /// The non-empty selections over the row `ls..le`, as chars of `text`.
    fn selected(&self, sels: &[Selection], ls: usize, le: usize) -> Vec<Range<usize>> {
        let from = sels.partition_point(|s| s.end() <= ls);
        (sels[from..].iter())
            .take_while(|s| s.start() < le)
            .filter(|s| !s.is_empty())
            .map(|s| self.char_of(s.start().max(ls))..self.char_of(s.end().min(le)))
            .collect()
    }

    /// The text as white space marks read it: a placeholder is no white space.
    fn marked(&self) -> String {
        let mut text = self.text.clone();
        // Byte offsets belong to the original row; replace the later ones first.
        for (_, _, b, fold) in self.pieces.iter().rev() {
            if *fold {
                let marker: String = std::iter::repeat_n(PLACEHOLDER, PLACEHOLDER_COLS).collect();
                text.replace_range(*b..b + PLACEHOLDER_COLS, &marker);
            }
        }
        text
    }
}

/// The char a clipped row starts at: it is drawn at that many columns.
fn clip_start(scroll_left: f32, space_w: f32) -> usize {
    ((scroll_left / space_w) as usize).saturating_sub(CLIP_MARGIN)
}

/// The scroll that keeps `x` at least `margin` inside a view `w` wide.
fn follow_left(x: f32, left: f32, w: f32, margin: f32) -> f32 {
    if x < left + margin {
        (x - margin).max(0.)
    } else if x > left + w - margin {
        x - w + margin
    } else {
        left
    }
}

/// The scroll that keeps the caret (char `cc`) of a clipped long line in view. `clip` draws
/// the row from char `a` at `a` columns, then at the glyphs' real widths: `width_from(a)` is
/// the caret's distance from char `a` (CJK and tabs make it more than a column a char), so
/// the caret lands where the row's start is picked, not at `cc` columns.
fn long_follow(cc: usize, left: f32, w: f32, margin: f32, space_w: f32, width_from: impl Fn(usize) -> f32) -> f32 {
    let drawn = |left: f32| {
        let a = clip_start(left, space_w).min(cc);
        a as f32 * space_w + width_from(a)
    };
    let x = drawn(left);
    if x >= left + margin && x <= left + w - margin {
        return left;
    }
    let target = if x < left + margin { margin } else { w - margin };
    // the row must start this far left of the caret, the clip margin included
    let want = target + CLIP_MARGIN as f32 * space_w;
    if width_from(0) <= want {
        // near the line's start the row is not cut: plain scrolling
        return (width_from(0) - target).max(0.);
    }
    // the last start far enough left; width_from shrinks as `a` grows
    let (mut lo, mut hi) = (0, cc);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if width_from(mid) >= want {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    // just past the float that floors to `lo + CLIP_MARGIN` in `clip_start`
    (lo + CLIP_MARGIN) as f32 * space_w + 0.01
}

/// Chars past a row's end that its white space looks at for text, to know whether it is
/// trailing. A row further than this from its line's end marks no trailing white
/// space, so a run of white space longer than this is not marked in the rows before its end;
/// look to the line's end if such runs matter.
const TRAIL_LOOK: usize = 256;

/// Unwrapped lines longer than this many chars are shaped only around the view.
pub(super) const LONG_LINE: usize = 4096;
/// Chars shaped either side of the view on such a line.
const CLIP_MARGIN: usize = 256;

/// A long unwrapped row cut to the chars around the view, so a 10 MB line is not shaped
/// whole every frame. Each display char counts as one column here (a placeholder its
/// columns), so tabs and wide chars left of the view shift where its text is drawn and
/// clicked; only rows over `LONG_LINE` display chars.
fn clip(mut row: Row, scroll_left: f32, width: f32, space_w: f32) -> Row {
    let n = display_len(&row.pieces);
    let a = clip_start(scroll_left, space_w).min(n);
    let b = (((scroll_left + width) / space_w) as usize + CLIP_MARGIN).min(n);
    row.last &= b == n;
    let (pieces, x0) = cut_row(&row.pieces, a, b);
    let chars = |p: &Piece| match p {
        Piece::Text(r) | Piece::Fold(r) => r.clone(),
    };
    row.chars = match (pieces.first(), pieces.last()) {
        (Some(f), Some(l)) => chars(f).start..chars(l).end,
        _ => row.chars.end..row.chars.end,
    };
    row.pieces = match pieces.is_empty() {
        true => vec![Piece::Text(row.chars.clone())],
        false => pieces,
    };
    row.x0 = x0;
    row
}

/// The display chars a row's pieces take.
pub(super) fn display_len(pieces: &[Piece]) -> usize {
    pieces.iter().map(piece_width).sum()
}

fn piece_width(p: &Piece) -> usize {
    match p {
        Piece::Text(r) => r.len(),
        Piece::Fold(_) => PLACEHOLDER_COLS,
    }
}

/// Display chars `a..b` of a row's pieces, a placeholder partly in them taken whole, and the
/// display char they start at.
fn cut_row(pieces: &[Piece], a: usize, b: usize) -> (Vec<Piece>, usize) {
    let (mut out, mut at, mut start) = (Vec::new(), 0, None);
    for p in pieces {
        let w = piece_width(p);
        let (s, e) = (at.max(a), (at + w).min(b));
        if s < e {
            match p {
                Piece::Text(r) => out.push(Piece::Text(r.start + s - at..r.start + e - at)),
                Piece::Fold(_) => out.push(p.clone()),
            }
            start.get_or_insert(if matches!(p, Piece::Fold(_)) { at } else { s });
        }
        at += w;
    }
    (out, start.unwrap_or(a))
}

/// The display char of a row's pieces that offset `o` is drawn before, as `RowText::char_of`
/// gives it, without the row's text.
fn char_in(pieces: &[Piece], o: usize) -> usize {
    let mut at = 0;
    for p in pieces {
        match p {
            Piece::Text(r) if o <= r.end => return at + o.saturating_sub(r.start),
            Piece::Fold(r) if o < r.end => return at,
            _ => at += piece_width(p),
        }
    }
    at
}

struct CursorColumns {
    xs: Vec<Pixels>,
    #[cfg(test)]
    shapes: usize,
    #[cfg(test)]
    chars: usize,
}

/// Each caret row is measured once, using this frame's shaping or its bounded clip path.
/// Only distinct positions intersecting the viewport are crossed with visible text rows.
fn cursor_columns(
    doc: &Document,
    selections: &[Selection],
    rows: &Padded<Folded>,
    layout: &Layout,
    scroll_left: f32,
    shape: impl Fn(SharedString) -> ShapedLine,
) -> CursorColumns {
    struct MeasuredRow {
        index: usize,
        row: Row,
        shaped: Option<(RowText, Vec<usize>, ShapedLine)>,
    }
    let mut out = CursorColumns {
        xs: Vec::new(),
        #[cfg(test)]
        shapes: 0,
        #[cfg(test)]
        chars: 0,
    };
    let mut cached: Option<MeasuredRow> = None;
    for sel in selections {
        let r = rows.shown_row(doc, rows.inner().row_for_offset(doc, sel.head)).0;
        if cached.as_ref().is_none_or(|c| c.index != r) {
            let Some(row) = row(doc, rows, r) else { continue };
            // Also bound offscreen rows of an unfinished lazy wrap map.
            let row = if display_len(&row.pieces) > LONG_LINE {
                clip(row, scroll_left, f32::from(layout.text_width), f32::from(layout.space_w))
            } else {
                row
            };
            cached = Some(MeasuredRow { index: r, row, shaped: None });
        }
        let Some(cached) = cached.as_mut() else { continue };
        let row = &cached.row;
        if sel.head < row.chars.start || sel.head > row.chars.end {
            continue;
        }
        let x = if sel.head == row.chars.start {
            layout.space_w * (row.x0 + sel.head_virtual) as f32
        } else {
            let (rt, cols, line) = cached.shaped.get_or_insert_with(|| {
                let rt = RowText::new(&doc.rope, &row.pieces);
                let (shown, cols) = expand_tabs_at(&rt.text, doc.tab_size, row.x0);
                let painted = r
                    .checked_sub(layout.first_row)
                    .and_then(|i| layout.rows.get(i)?.as_ref())
                    .filter(|(x0, line)| *x0 == row.x0 && line.text.as_ref() == shown.as_ref());
                let line = match painted {
                    Some((_, line)) => line.clone(),
                    None => {
                        #[cfg(test)]
                        {
                            out.shapes += 1;
                            out.chars += shown.chars().count();
                        }
                        shape(shown.into_owned().into())
                    }
                };
                (rt, cols, line)
            });
            layout.space_w * (row.x0 + sel.head_virtual) as f32
                + line.x_for_index(byte_of(&line.text, cols[rt.char_of(sel.head)]))
        } + layout.text_left;
        if x < layout.text_area.right() && x + layout.space_w > layout.text_area.left() {
            out.xs.push(x);
        }
    }
    out.xs.sort_unstable_by(|a, b| f32::from(*a).total_cmp(&f32::from(*b)));
    out.xs.dedup_by(|a, b| f32::from(*a - *b).abs() < 0.1);
    out
}

impl IntoElement for EditorElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let run = |len: usize, color| text_run(len, &font, color);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        // `text` in one colour
        let shape = |text: SharedString, color| {
            let runs = [run(text.len(), color)];
            window.text_system().shape_line(text, font_size, &runs, None)
        };
        let space_w = shape("    ".into(), style.color).width / 4.;
        self.editor.update(cx, |editor, cx| {
            editor.clear_stale_occurrences(cx);
            let spell_active = editor.sync_spelling(cx);
            let doc = doc_of(&editor.document, cx);
            let mut spell_visible = Vec::new();
            let single = editor.single_line;
            let lh_f = editor.line_height();
            let lh64 = lh_f as f64;
            let lh = px(lh_f);
            let mm_lh = editor.minimap_line_height();
            let mm_cw = f32::from(space_w) * theme::MINIMAP_CHAR;
            // Sublime's gutter fits the line count's digits (two at least)
            let show_gutter = editor.view.gutter;
            let (gutter, mut margin) = match single {
                true => (px(0.), px(0.)),
                false if !show_gutter => (px(0.), px(theme::TEXT_MARGIN)),
                false => {
                    let digits = doc.line_count().to_string().len().max(2);
                    let w = px(theme::GUTTER_LEFT + theme::GUTTER_RIGHT) + space_w * digits as f32;
                    (w, px(theme::TEXT_MARGIN))
                }
            };
            let mm_w = match !single && editor.view.minimap {
                true => (bounds.size.width - gutter) * theme::MINIMAP_SHARE,
                false => px(0.),
            };
            // the columns a row wraps at in a view `w` wide: a fixed column, or the view's width
            // when that is narrower
            let wrap_cols = editor.wrap_cols();
            let fit = |w: Pixels| match wrap_width(f32::from(w), f32::from(space_w)) {
                fit if wrap_cols == 0 => fit,
                fit => fit.min(wrap_cols),
            };
            // distraction-free: the wrapped column in the middle of the view
            if !single && editor.view.centered && editor.wraps() {
                let view = bounds.size.width - gutter - margin - mm_w;
                let used = space_w * (fit(view) + 2) as f32;
                if view > used {
                    margin += (view - used) / 2.;
                }
            }
            let view = bounds.size.width - gutter - margin - mm_w;
            let text_width = view - px(8.);
            let want = (editor.wraps()).then(|| (fit(view), doc.tab_size));
            let current = match (&editor.wrap, want) {
                (Some(w), Some((width, tab))) => w.fits(doc, width, tab),
                (None, None) => true,
                _ => false,
            };
            let settling = editor.wrap.as_ref().is_some_and(|w| !w.settled());
            // the text on top stays on top, unless a change already pinned what to keep
            if (!current || settling) && editor.top_anchor.is_none() {
                editor.top_anchor = Some(top_anchor(&editor.rows(), doc, editor.scroll_top / lh64));
            }
            if !current {
                editor.wrap = want.map(|(width, tab)| match editor.wrap.take() {
                    // a resize keeps the old rows until each line is re-wrapped below
                    Some(mut w) if w.lines() == doc.line_count() => {
                        w.resize(width, tab);
                        w
                    }
                    _ => WrapMap::lazy(doc, width, tab, &editor.folds),
                });
            }
            let height = f32::from(bounds.size.height);
            if let Some(w) = editor.wrap.as_mut().filter(|w| !w.settled()) {
                // what this frame paints and hit-tests is wrapped now: the lines around the top
                // text and around the caret it may scroll to, as many as the minimap shows
                let span = (height / mm_lh) as usize + 2;
                let state = read_of(&editor.document, &editor.state, cx);
                let around = editor.top_anchor.map(|t| t.0);
                let caret = editor.follow_caret.then(|| state.cursor());
                for o in around.into_iter().chain(caret) {
                    let line = state.point(o).0;
                    w.rewrap(state.doc, line.saturating_sub(span)..line + span, &editor.folds);
                }
                // the rest a slice per frame
                let deadline = Instant::now() + Duration::from_millis(4);
                if !w.settle(state.doc, deadline, &editor.folds) {
                    window.request_animation_frame();
                }
            }
            if editor.wrap.is_some() && editor.state.selections[editor.state.primary()].head_virtual == 0 {
                editor.scroll_left = 0.;
            }
            // a multiline field scrolls to its caret; a one-row field never scrolls down
            let fixed = editor.fixed_rows();
            if let Some(top) = editor.top_anchor.take().filter(|_| !single) {
                editor.scroll_top = top_row(&editor.rows(), doc, top) * lh64;
            }
            let state = read_of(&editor.document, &editor.state, cx);
            let wrap = editor.wrap.as_ref();
            let folded = view_rows(wrap, &editor.folds, &editor.fillers);
            let count = folded.rows(state.doc);
            let total = count as f64 * lh64;
            let follow = std::mem::take(&mut editor.follow_caret);
            // again with this frame's rows: a re-wrap since the edit (a new width, the map
            // built just now) moves the caret's row
            let caret_row = follow.then(|| folded.row_of(state.doc, state.cursor()).row.0);
            if let Some(row) = caret_row.filter(|_| !fixed) {
                let y = row as f64 * lh64;
                if y < editor.scroll_top {
                    editor.scroll_top = y;
                } else if y + lh64 > editor.scroll_top + height as f64 {
                    editor.scroll_top = y + lh64 - height as f64;
                }
            }
            // a re-wrap or a file shrunk on disk can leave the view past the end
            if !fixed {
                editor.scroll_top = editor.scroll_top.min((total - height as f64).max(0.));
            }
            // a page glide draws on its way, a frame at a time
            if (editor.scroll_anim).is_some_and(|(_, t0)| t0.elapsed() >= crate::editor::SCROLL_ANIM) {
                editor.scroll_anim = None;
            }
            if editor.scroll_anim.is_some() {
                window.request_animation_frame();
            }
            let scroll_top = if fixed { 0. } else { editor.drawn_scroll() };
            let first = (scroll_top / lh64).floor() as usize;
            let visible = (height / lh_f).ceil() as usize + 1;
            let last = (first + visible).min(count);
            if follow && (wrap.is_none() || state.view.selections[state.primary()].head_virtual > 0) {
                let cl = state.point(state.cursor()).0;
                let (w, margin) = (f32::from(text_width), 40.);
                if caret_row.is_some_and(|r| (first..last).contains(&r)) {
                    // the caret's display line as drawn: its line, or the lines its folds join
                    // with each fold a placeholder; the caret's display char in it
                    let caret = caret_row.and_then(|r| row(state.doc, &folded, r));
                    // a wrapped continuation row is drawn from its indent, its tabs counted from it
                    let x0 = caret.as_ref().map_or(0, |row| row.x0);
                    let pieces = caret
                        .map_or_else(|| vec![Piece::Text(state.line_start(cl)..state.line_end(cl))], |row| row.pieces);
                    let cc = char_in(&pieces, state.cursor());
                    if display_len(&pieces) > LONG_LINE {
                        // one shaping of the chars that can reach the view left of the caret
                        let sw = f32::from(space_w);
                        let reach = ((w + CLIP_MARGIN as f32 * sw) / sw) as usize * 4 + 64;
                        let (cut, lo) = cut_row(&pieces, cc.saturating_sub(reach), cc);
                        let text = RowText::new(&state.doc.rope, &cut).text;
                        let (shown, cols) = expand_tabs_at(&text, state.doc.tab_size, lo);
                        let shaped = shape(shown.to_string().into(), style.color);
                        let x_of = |i: usize| f32::from(shaped.x_for_index(byte_of(&shown, cols[i])));
                        let gap = state.view.selections[state.primary()].head_virtual;
                        let end = x_of(cc - lo) + sw * gap as f32;
                        // chars left of the shaped ones count a column each
                        let width_from = |a: usize| {
                            end - x_of(a.clamp(lo, cc) - lo) + lo.saturating_sub(a) as f32 * sw
                                - a.saturating_sub(cc) as f32 * sw
                        };
                        editor.scroll_left = long_follow(cc + gap, editor.scroll_left, w, margin, sw, width_from);
                    } else {
                        let text = RowText::new(&state.doc.rope, &pieces).text;
                        let (shown, cols) = expand_tabs_at(&text, state.doc.tab_size, x0);
                        let shaped = shape(shown.to_string().into(), style.color);
                        let x = f32::from(shaped.x_for_index(byte_of(&shown, cols[cc.min(cols.len() - 1)])))
                            + f32::from(space_w) * (x0 + state.view.selections[state.primary()].head_virtual) as f32;
                        editor.scroll_left = follow_left(x, editor.scroll_left, w, margin);
                    }
                }
            }
            let text_left = bounds.left() + gutter + margin - px(editor.scroll_left);
            // the lines with a caret, of the selections on screen only (1,000 carets cost a
            // line lookup each, every frame)
            let (a, b) = {
                let line = |r: usize| line_near(state.doc, &folded, r);
                // to the end of the last row's display line, past its folds
                let end = editor.folds.display_lines(line(last)).1;
                (state.line_start(line(first)), state.line_end(end))
            };
            let on_screen = {
                let sels = &state.view.selections;
                let from = sels.partition_point(|s| s.end() < a);
                let to = from + sels[from..].partition_point(|s| s.start() <= b);
                &sels[from..to]
            };
            // the first line of each caret's display line, as rows go by
            let mut cursor_lines: Vec<usize> =
                (on_screen.iter()).map(|s| editor.folds.display_lines(state.point(s.head).0).0).collect();
            cursor_lines.sort_unstable();
            let primary_line = state.doc.rope.char_to_line(state.cursor());
            // where the text being dragged would drop, once the drag has begun
            let drop_at = (editor.drag.as_ref())
                .filter(|d| d.text.as_ref().is_some_and(|t| t.moved))
                .and_then(|d| editor.drop_offset(d.pos, cx));
            let marked = editor.marked.clone();
            let (caret_color, selection_color) = if single {
                (theme::ui(theme::WIDGET_CARET), theme::ui(theme::WIDGET_SELECTION))
            } else {
                (c(theme::CARET), c(theme::SELECTION))
            };
            let selected_fg = (!single).then(theme::selection_foreground).flatten();
            // the fold buttons, in the 20 pt between the numbers and the text; when they
            // fade, ▾ shows only while the pointer is over the gutter
            let gutter_bounds = Bounds::new(bounds.origin, size(gutter, bounds.size.height));
            let buttons = !single && show_gutter && editor.view.fold_buttons;
            let over_gutter = editor.hover.is_some_and(|h| gutter_bounds.contains(&h));
            let fade = editor.view.fade_fold_buttons && !over_gutter;
            let background = editor.background();
            let tint = |alpha| gpui::hsla(0., 0., if background.l > 0.5 { 0. } else { 1. }, alpha);
            #[cfg(test)]
            let mut drawn = Drawn::default();
            let mut p = Prepaint {
                hitbox,
                lines: Vec::with_capacity(last.saturating_sub(first)),
                gutter_right: bounds.left() + gutter - px(theme::GUTTER_NUMBER_RIGHT),
                text_mask: Bounds::from_corners(
                    point(bounds.left() + gutter, bounds.top()),
                    point(bounds.right() - mm_w, bounds.bottom()),
                ),
                gutter: Vec::new(),
                change_hits: Vec::new(),
                changes_version: state.doc.version,
                fold_buttons: Vec::new(),
                fold_hits: Vec::new(),
                cursor_columns: Vec::new(),
                below: Vec::new(),
                carets: Vec::new(),
                outlines: Vec::new(),
                marks: Vec::new(),
                inline_diagnostics: Vec::new(),
                minimap: Vec::new(),
                minimap_bounds: None,
                gutter_bounds,
                scrollbar: None,
                shadow: None,
            };
            let mut rows = Vec::with_capacity(last.saturating_sub(first));
            // Find Results' outlines are the buffer's (undo brings them back), the find panel's
            // the view's
            let outlines = match editor.results {
                true => &state.doc.outlines,
                false => &editor.find_ranges,
            };
            // the rows of one line share its colours
            let mut cached: Option<(usize, Vec<Run>)> = None;
            // the first and last lines past what this frame may parse
            let mut unready: Option<(usize, usize)> = None;
            editor.highlighter.begin_frame();
            // (line, indent guides): a wrapped line's rows share them too
            let mut guides = (usize::MAX, 0);
            // (selection index, its piece of a row), in row order, for the borders
            let mut sel_rects: Vec<(usize, SelRect)> = Vec::new();
            // a row runs on past the text's right edge
            let mut cut_off = false;
            let language_current = editor.language_current(cx);
            let inline_diagnostics = editor.inline_diagnostics.as_ref().filter(|d| {
                !single
                    && editor.view.inline_diagnostics
                    && language_current
                    && marked.is_empty()
                    && d.current(editor, cx)
            });
            let mut semantic_styles = std::collections::HashMap::new();
            // the diagnostics on screen, for each row to look through
            let diagnostics: Vec<_> =
                (editor.diagnostics.iter()).filter(|d| d.range.start <= b && d.range.end >= a).collect();
            // white space is marked in the indent guides' colour (see `mark_colors`), each
            // kind of mark shaped the first time it is drawn
            let white = if single { WhiteSpace::NONE } else { editor.view.white_space };
            let (guide, on_selection) = mark_colors(c(theme::GUIDE), c(theme::EDITOR_FG));
            // by whether the mark is on selected text, then its kind: the colour and the glyph
            let mut glyphs: [[Option<(Hsla, ShapedLine)>; 4]; 2] = Default::default();
            let warning_colors = [
                warning_color(background),
                warning_color(background.blend(selection_color)),
                warning_color(background.blend(c(theme::FIND_CURRENT_BG))),
            ];
            let mut warning_glyphs = std::collections::HashMap::new();
            for r in first..last {
                // the difference in f64, then a screen's worth of pixels in f32
                let y = bounds.top() + px((r as f64 * lh64 - scroll_top) as f32);
                // the row from `x0` to `x1`
                let span = |x0: Pixels, x1: Pixels| Bounds::from_corners(point(x0, y), point(x1, y + lh));
                let Some(row) = row(state.doc, &folded, r) else {
                    // a filler row: hatched across the text, no number, no text to select
                    let across = span(p.text_mask.left(), p.text_mask.right());
                    let hatch = gpui::pattern_slash(
                        tint(theme::FILLER_HATCH),
                        theme::FILLER_HATCH_LINE,
                        theme::FILLER_HATCH_GAP,
                    );
                    p.below.push(fill(across, hatch));
                    #[cfg(test)]
                    drawn.fillers.push(r);
                    rows.push(None);
                    continue;
                };
                // a row is long for what it shows, not for what its folds hide
                let row = match row {
                    row if wrap.is_none() && display_len(&row.pieces) > LONG_LINE => {
                        clip(row, editor.scroll_left, f32::from(text_width), f32::from(space_w))
                    }
                    row => row,
                };
                let (line, ls, le) = (row.line, row.chars.start, row.chars.end);
                // the row's text: each piece in its own line's colours (a display line runs on
                // over the lines its folds join), each placeholder blank under its marker
                let rt = RowText::new(&state.doc.rope, &row.pieces);
                let mut runs = rt.runs(&state.doc.rope, style.color, run, &mut cached, |line| {
                    line_colors(&mut editor.highlighter, &state.doc.rope, line, single, &mut unready)
                });
                let text = &rt.text;
                // a buffer offset's byte in the row's text, clamped to the row
                let rb = |o: usize| rt.byte_of(&state.doc.rope, o.clamp(ls, le));
                if language_current {
                    let from = editor.semantic_spans.partition_point(|s| s.range.end <= ls);
                    for span in editor.semantic_spans[from..].iter().take_while(|s| s.range.start < le) {
                        let style = semantic_styles.entry(span.kind.as_str()).or_insert_with(|| {
                            crate::languages::language::semantic_scope(&span.kind)
                                .map(crate::languages::highlight::scope_style)
                        });
                        if let Some((color, look)) = *style {
                            runs = restyle(runs, rb(span.range.start), rb(span.range.end), |r| {
                                *r = styled(r.clone(), look);
                                r.color = color;
                            });
                        }
                    }
                    for (diagnostic, Range { start, end }) in
                        (diagnostics.iter()).filter_map(|d| Some((d, diagnostic_on_row(&d.range, ls, le)?)))
                    {
                        let color = c(if diagnostic.severity == 1 { 0xec5f66 } else { 0xf9ae58 });
                        runs = restyle(runs, rb(start), rb(end), |r| r.underline = underline(color, true));
                        p.gutter.push(fill(
                            Bounds::new(point(p.gutter_right - px(3.), y + px(3.)), size(px(2.), lh - px(6.))),
                            color,
                        ));
                    }
                }
                if spell_active {
                    for (r, _, _, fold) in &rt.pieces {
                        if *fold {
                            continue;
                        }
                        let l = state.doc.rope.char_to_line(r.start);
                        let base = state.line_start(l);
                        spell_visible
                            .push(crate::languages::spell::Region { line: l, chars: r.start - base..r.end - base });
                        #[cfg(test)]
                        drawn.spelling.extend(editor.spell_cache.ranges(l).filter_map(|w| {
                            let (start, end) = ((base + w.start).max(r.start), (base + w.end).min(r.end));
                            (start < end).then_some(start..end)
                        }));
                        runs = spell_underlines(
                            runs,
                            &state.doc.rope,
                            &rt,
                            r.clone(),
                            (editor.spell_cache.ranges(l)).map(|w| base + w.start..base + w.end),
                        );
                    }
                }
                // a selection on one of the find panel's matches is Sublime's current match
                let current = |s: &Selection| {
                    !editor.results
                        && !s.is_empty()
                        && outlines.binary_search_by(|r| (r.start, r.end).cmp(&(s.start(), s.end()))).is_ok()
                };
                let sels = &state.view.selections;
                let from = sels.partition_point(|s| s.end() < ls);
                let recolored: Vec<(usize, usize, Hsla)> = (sels[from..].iter().take_while(|s| s.start() <= le))
                    .filter_map(|sel| {
                        let color = match current(sel) {
                            true => Some(c(theme::FIND_CURRENT_FG)),
                            false => selected_fg.filter(|_| !sel.is_empty()),
                        };
                        color.map(|fg| (rb(sel.start()), rb(sel.end()), fg))
                    })
                    .collect();
                if !recolored.is_empty() {
                    runs = recolor_each(runs, &recolored);
                }
                // IME composition: only the marked text is underlined
                for m in marked.iter().filter(|m| m.start < le && m.end > ls) {
                    runs = restyle(runs, rb(m.start), rb(m.end), |r| r.underline = underline(r.color, false));
                }
                let (shown, cols) = expand_tabs_at(text, state.doc.tab_size, row.x0);
                if let Cow::Owned(_) = shown {
                    expand_runs(text, &cols, &mut runs);
                }
                let shaped =
                    window.text_system().shape_line(SharedString::from(shown.to_string()), font_size, &runs, None);
                let row_left = text_left + space_w * row.x0 as f32;
                // every char's x in one pass over the glyphs, made when first asked: each caret,
                // selection edge and mark would otherwise scan the row to its char, twice
                let row_xs = std::cell::OnceCell::new();
                let xs_of_row = || row_xs.get_or_init(|| char_xs(&shown, &cols, &shaped));
                let x_of_char = |i: usize| {
                    let xs = xs_of_row();
                    row_left + xs[i.min(xs.len() - 1)]
                };
                let x_at = |off: usize| x_of_char(rt.char_of(off));
                // Only the terminal physical line's visible end gets an overlay. Hidden ends,
                // continuation rows and horizontally clipped tails never acquire synthetic rows.
                if row.last {
                    if let (Some(cache), Some(Piece::Text(tail))) = (inline_diagnostics, row.pieces.last()) {
                        let physical_line = state.doc.rope.char_to_line(tail.end);
                        let index = cache.winners.partition_point(|d| d.line < physical_line);
                        let source_end = x_at(tail.end);
                        if let Some(diagnostic) = cache.winners.get(index).filter(|d| {
                            d.line == physical_line
                                && tail.end == state.line_end(physical_line)
                                && d.severity <= editor.view.inline_diagnostics_severity as u64
                                && source_end >= p.text_mask.left()
                                && source_end < p.text_mask.right()
                        }) {
                            let virtual_end = on_screen
                                .iter()
                                .flat_map(|s| {
                                    [
                                        if s.head == tail.end { s.head_virtual } else { 0 },
                                        if s.anchor == tail.end { s.anchor_virtual } else { 0 },
                                    ]
                                })
                                .max()
                                .unwrap_or(0);
                            let x = source_end + space_w * (virtual_end + 2) as f32;
                            let width = p.text_mask.right() - space_w - x;
                            let color = crate::chrome::theme::ca(match diagnostic.severity {
                                1 => 0xec5f66aa,
                                2 => 0xf9ae58aa,
                                _ => 0x99aabb99,
                            });
                            if let Some(message) = elide_inline(&diagnostic.text, width, |text| shape(text, color)) {
                                #[cfg(test)]
                                drawn.inline_diagnostics.push((
                                    physical_line,
                                    diagnostic.index,
                                    message.text.to_string(),
                                    Bounds::new(point(x, y), size(message.width, lh)),
                                ));
                                p.inline_diagnostics.push((message, point(x, y)));
                            }
                        }
                    }
                }
                let on_cursor_line = cursor_lines.binary_search(&line).is_ok();
                if !single && on_cursor_line {
                    #[cfg(test)]
                    drawn.current.push(r);
                    p.gutter
                        .push(fill(Bounds::new(point(bounds.left(), y), size(gutter, lh)), theme::line_highlight()));
                }
                if !single && show_gutter && editor.changes_version == state.doc.version {
                    if let Some((index, marker)) = change_marker(&editor.changes, line, state.line_count()) {
                        let (color, top, height, width, draw) = match marker {
                            ChangeMarker::Added => (0x99c794, y, lh, px(3.), row.first),
                            ChangeMarker::Modified => (0x6699cc, y, lh, px(3.), row.first),
                            ChangeMarker::Deleted(false) => (0xec5f67, y, px(2.), px(10.), row.first),
                            ChangeMarker::Deleted(true) => (0xec5f67, y + lh - px(2.), px(2.), px(10.), row.last),
                        };
                        if draw {
                            p.change_hits.push((
                                Bounds::new(point(bounds.left(), y), size(px(7.), lh)),
                                index,
                                editor.changes[index].clone(),
                            ));
                            p.gutter.push(fill(
                                Bounds::new(point(bounds.left() + px(2.), top), size(width, height)),
                                c(color),
                            ));
                        }
                    }
                }
                if !single
                    && row.first
                    && state.view.bookmarks.binary_search(&state.line_start(line)).is_ok()
                    && show_gutter
                {
                    // Sublime's gutter bookmark icon, as a dot left of the line number
                    #[cfg(test)]
                    BOOKMARK_DOTS.set(BOOKMARK_DOTS.get() + 1);
                    let d = px(7.);
                    let dot = Bounds::new(point(bounds.left() + px(8.), y + (lh - d) / 2.), size(d, d));
                    p.gutter.push(rounded(dot, d / 2., c(theme::CARET)));
                }
                // ▾ beside a line that heads a region, ▸ beside a folded one, whose hidden
                // text shows as Sublime's marker after its own; a click on either toggles it
                let folded_here = !single && editor.folds.is_folded(line);
                if buttons && row.first && (folded_here || editor.fold_header(state.doc, line)) {
                    let hit = Bounds::new(point(bounds.left() + gutter - px(20.), y), size(px(20.), lh));
                    p.fold_hits.push((hit, FoldHit::Toggle(line)));
                    if folded_here || !fade {
                        let color = match editor.hover.is_some_and(|h| hit.contains(&h)) {
                            true => tint(theme::FOLD_BUTTON_HOVER),
                            false => tint(theme::FOLD_BUTTON),
                        };
                        p.fold_buttons.push((hit, folded_here, color));
                        #[cfg(test)]
                        drawn.fold_buttons.push((line, folded_here));
                    }
                }
                // each fold's placeholder: fold.png's pill with three dots (18 × 10 where its
                // columns give room) over its blank columns; a click on it opens the fold
                for (r, at, _, fold) in &rt.pieces {
                    if !fold {
                        continue;
                    }
                    let (x0, x1) = (x_of_char(*at), x_of_char(at + PLACEHOLDER_COLS));
                    // scrolled sideways under the gutter or the minimap, it takes no clicks there
                    let hit = span(x0, x1).intersect(&p.text_mask);
                    if hit.size.width > px(0.) {
                        p.fold_hits.push((hit, FoldHit::Open(r.start)));
                        #[cfg(test)]
                        drawn.markers.push((line, hit));
                    }
                    let w = (x1 - x0 - px(2.)).clamp(px(8.), px(18.));
                    let pill = Bounds::new(point(x0 + (x1 - x0 - w) / 2., y + (lh - px(10.)) / 2.), size(w, px(10.)));
                    p.outlines.push(rounded(pill, px(2.), tint(theme::FOLD_BUTTON)));
                    let gap = (w - px(6.)) / 4.;
                    for i in 0..3 {
                        let dx = gap * (i + 1) as f32 + px(2. * i as f32);
                        let dot = point(pill.left() + dx, pill.top() + px(4.));
                        p.outlines.push(fill(Bounds::new(dot, size(px(2.), px(2.))), background));
                    }
                }
                for &col in editor.view.rulers.iter().filter(|_| !single) {
                    p.below.push(fill(
                        Bounds::new(point(text_left + space_w * col as f32, y), size(px(1.), lh)),
                        c(theme::GUIDE),
                    ));
                }
                if !single {
                    if guides.0 != line {
                        guides = (line, guide_levels(state.doc, line));
                    }
                    // Sublime's dotted indent guides, one per enclosing indent level
                    for level in 0..guides.1 {
                        let x = text_left + space_w * (level * state.doc.tab_size) as f32 - px(1.5);
                        for dy in (0..lh_f as i32).step_by(2) {
                            p.below.push(fill(
                                Bounds::new(point(x, y + px(dy as f32)), size(px(1.), px(1.))),
                                c(theme::GUIDE),
                            ));
                        }
                    }
                }
                if let Some(comparison) = &editor.comparison {
                    let rgb = if comparison.added { 0x99c794 } else { 0xec5f67 };
                    for (ranges, inline) in [(&comparison.lines, false), (&comparison.inline, true)] {
                        let from = ranges.partition_point(|r| {
                            #[cfg(test)]
                            {
                                drawn.comparison_visits += 1;
                            }
                            r.end <= ls
                        });
                        for range in ranges[from..].iter().take_while(|r| r.start <= le) {
                            #[cfg(test)]
                            {
                                drawn.comparison_visits += 1;
                            }
                            let row_end = le + usize::from(!inline && row.last && le < state.len());
                            let (start, end) = (range.start.max(ls), range.end.min(row_end));
                            if start >= end {
                                continue;
                            }
                            let (x0, x1) = if inline {
                                (x_at(start), x_at(end))
                            } else {
                                (p.text_mask.origin.x, p.text_mask.right())
                            };
                            let bounds = span(x0, x1);
                            let color = crate::chrome::theme::ca((rgb << 8) | if inline { 0x66 } else { 0x24 });
                            p.below.push(fill(bounds, color));
                            #[cfg(test)]
                            drawn.comparison.push((start..end, bounds, color, inline));
                        }
                    }
                }
                // a folded line's hidden text is in its marker: the row draws through the fold's
                // end (`re`), at the marker's right (`rx`) past its own text
                // the row's last offset (past a fold it ends with) and the x past its text
                let (re, rx) = (le, row_left + shaped.width);
                let x_of = |off: usize| if off > le { rx } else { x_at(off) };
                // where the chars `s..e` are drawn across the row, if any of them are on it: a span
                // that swallows the line break shows a small box past the end; one running on over
                // a soft break fills to the row's end
                let across = |s: usize, e: usize| {
                    if e <= ls || s > re || !row.last && s == le {
                        return None;
                    }
                    let x0 = if s <= ls { row_left } else { x_of(s) };
                    let x1 = match (e > re, row.last) {
                        (true, true) => rx + px(7.),
                        (true, false) => rx,
                        _ => x_of(e),
                    };
                    Some((x0, x1))
                };
                // Cached passive matches only cross visible source pieces, never fold placeholders.
                if let Some(occurrences) = &editor.occurrences {
                    for (piece, _, _, fold) in &rt.pieces {
                        if *fold {
                            continue;
                        }
                        let from = occurrences.ranges.partition_point(|r| {
                            #[cfg(test)]
                            {
                                drawn.occurrence_visits += 1;
                            }
                            r.end <= piece.start
                        });
                        for range in occurrences.ranges[from..].iter().take_while(|r| {
                            #[cfg(test)]
                            {
                                drawn.occurrence_visits += 1;
                            }
                            r.start < piece.end
                        }) {
                            let visible = range.start.max(piece.start)..range.end.min(piece.end);
                            let bounds = span(x_at(visible.start), x_at(visible.end));
                            p.below.push(fill(bounds, tint(0.10)));
                            #[cfg(test)]
                            drawn.occurrences.push((visible, bounds));
                        }
                    }
                }
                // the highlight styles, under the selections and the find outlines
                for (style, ranges) in state.view.highlights.iter().enumerate() {
                    let from = ranges.partition_point(|r| r.end <= ls);
                    let color = crate::chrome::theme::ca(theme::HIGHLIGHT_STYLES[style]);
                    for (x0, x1) in
                        (ranges[from..].iter()).take_while(|r| r.start <= le).filter_map(|r| across(r.start, r.end))
                    {
                        p.below.push(fill(span(x0, x1), color));
                    }
                }
                // a soft break's offset belongs to the upper row: the lower one starts after it
                let on_row = |off: usize| off <= re && (off > ls || row.first && off == ls);
                let extra = px(if single { 0. } else { theme::CARET_EXTRA });
                // selections and matches are sorted and disjoint: skip straight to this row's
                let here = sels[from..].iter().take_while(|s| s.start() <= re);
                for (i, sel) in here.clone().enumerate() {
                    let virtual_span = row.last && sel.has_virtual() && sel.start() == re;
                    let Some((mut x0, mut x1)) = across(sel.start(), sel.end())
                        .or_else(|| virtual_span.then_some((rx, rx)))
                        .filter(|_| !sel.is_empty())
                    else {
                        continue;
                    };
                    if row.last {
                        if sel.start() == re {
                            x0 += space_w * sel.start_point().1 as f32;
                        }
                        if sel.end() == re {
                            x1 += space_w * sel.end_point().1 as f32;
                        }
                    }
                    if current(sel) {
                        p.below.push(fill(span(x0, x1), c(theme::FIND_CURRENT_BG)));
                        continue;
                    }
                    p.below.push(fill(span(x0, x1), selection_color));
                    #[cfg(test)]
                    {
                        drawn.selected.push(r);
                        drawn.selected_bounds.push((r, span(x0, x1)));
                    }
                    if !single {
                        let (first_row, last_row) = (sel.start() >= ls, sel.end() <= re);
                        let rect = (f32::from(x0), f32::from(x1), f32::from(y), first_row, last_row);
                        sel_rects.push((from + i, rect));
                    }
                }
                // the carets, then where dragged text would drop; Sublime's caret starts at the
                // char's left edge
                let heads = here.filter(|s| s.is_empty()).map(|s| (s.head, s.head_virtual));
                for (at, gap) in heads.chain(drop_at.map(|at| (at, 0))).filter(|&(o, _)| on_row(o)) {
                    let caret =
                        Bounds::new(point(x_of(at) + space_w * gap as f32, y - extra), size(px(2.), lh + extra * 2.));
                    p.carets.push(fill(caret, caret_color));
                    #[cfg(test)]
                    drawn.carets.push(caret);
                }
                // this row's white space; the line end of a folded line is after its marker. A
                // row with folds marks its own text only: its white space is trailing when the
                // display line ends with it
                if white.any() {
                    let folded_row = row.pieces.iter().any(|p| matches!(p, Piece::Fold(_)));
                    // the line the row's text is on: a wrapped display line's rows after a
                    // fold are on a line below its first
                    let line_end = state.line_end(state.doc.rope.char_to_line(ls));
                    let ahead = match folded_row {
                        true => le,
                        false => (le + TRAIL_LOOK).min(line_end),
                    };
                    let picked = match (white.selection, folded_row) {
                        (false, _) => Vec::new(),
                        (true, false) => selected_in(&state.view.selections, ls, ahead),
                        (true, true) => rt.selected(&state.view.selections, ls, le),
                    };
                    if white.all || white.trailing || !picked.is_empty() {
                        let here = WhiteSpace {
                            trailing: white.trailing && if folded_row { row.last } else { ahead == line_end },
                            ..white
                        };
                        let tail = match folded_row {
                            true => rt.marked(),
                            false => state.doc.rope.slice(ls..ahead).to_string(),
                        };
                        // a folded header's line break is the one after its fold's end
                        let eol = row.last && re < state.len();
                        // where `x_at` puts each char; only the chars in view are marked, so a
                        // 4,000-char row selected whole costs a screen's width of marks
                        let xs = xs_of_row();
                        let (view_l, view_r) = (p.text_mask.left(), p.text_mask.right());
                        let seen = xs.partition_point(|&x| row_left + x <= view_l).saturating_sub(1)
                            ..(xs.partition_point(|&x| row_left + x < view_r)).min(text.chars().count());
                        for (i, mark) in marks(&tail, seen, &picked, here, eol) {
                            let (left, right) = match mark {
                                Mark::Eol => (rx, rx + space_w),
                                _ => (row_left + xs[i], row_left + xs[i + 1]),
                            };
                            // a line end is on selected text when its break is
                            let at = match mark {
                                Mark::Eol => re,
                                _ => rt.offset_of(i),
                            };
                            // the find panel's current match is filled, not selected: the guide
                            // colour reads on its fill (5.1:1, the mix 1.9:1)
                            let inside = selected_at(&state.view.selections, at).is_some_and(|s| !current(s));
                            let (color, glyph) = glyphs[inside as usize][mark as usize].get_or_insert_with(|| {
                                let color = if inside { on_selection } else { guide };
                                (color, shape(mark.glyph().into(), color))
                            });
                            // a space is centred in its cell, a tab's arrow starts it
                            let x = match mark {
                                Mark::Space | Mark::Other => left + (right - left - glyph.width) / 2.,
                                Mark::Tab | Mark::Eol => left,
                            };
                            p.marks.push((glyph.clone(), point(x, y)));
                            if mark == Mark::Tab && right - left - glyph.width >= px(1.) {
                                // a thin line from the arrow to the next tab stop, down the row as
                                // Menlo's arrow runs (measured: 8.4 of 15 pt)
                                let mid = y + px((lh_f * 0.56).floor());
                                let line = fill(
                                    Bounds::from_corners(point(left + glyph.width, mid), point(right, mid + px(1.))),
                                    *color,
                                );
                                #[cfg(test)]
                                drawn.tab_lines.push(line.clone());
                                p.outlines.push(line);
                            }
                            #[cfg(test)]
                            drawn.marks.push((at, mark, span(left, right), *color));
                        }
                    }
                }
                // Walk only the clipped text pieces, never a whole physical line or hidden fold.
                // These glyphs are paint overlays: the shaped row and every position stay intact.
                if !single
                    && editor.view.unicode_warnings
                    && !(editor.view.unicode_warning_characters.is_empty()
                        && editor.view.unicode_warning_ranges.is_empty())
                {
                    for (piece, char_start, _, hidden) in &rt.pieces {
                        if *hidden {
                            continue;
                        }
                        for (i, ch) in state.doc.rope.slice(piece.clone()).chars().enumerate() {
                            #[cfg(test)]
                            {
                                drawn.unicode_visits += 1;
                            }
                            let Some(warning) = crate::config::settings::unicode::warning_for(
                                ch,
                                &editor.view.unicode_warning_characters,
                                &editor.view.unicode_warning_ranges,
                            ) else {
                                continue;
                            };
                            let at = piece.start + i;
                            if marked.iter().any(|m| m.contains(&at)) {
                                continue;
                            }
                            let xs = xs_of_row();
                            let left = row_left + xs[char_start + i];
                            let right = (row_left + xs[char_start + i + 1]).max(left + space_w);
                            if right <= p.text_mask.left() || left >= p.text_mask.right() {
                                continue;
                            }
                            let surface = selected_at(sels, at).map_or(0, |sel| if current(sel) { 2 } else { 1 });
                            let glyph = warning_glyphs.entry((warning.glyph, surface)).or_insert_with(|| {
                                let text: SharedString = warning.glyph.to_string().into();
                                let color = warning_colors[surface];
                                let run = text_run(text.len(), &font, color);
                                (color, window.text_system().shape_line(text, font_size * 0.65, &[run], None))
                            });
                            p.marks.push((glyph.1.clone(), point(left, y)));
                            #[cfg(test)]
                            {
                                drawn.unicode_warnings.push((at, warning.glyph, span(left, right)));
                                drawn.unicode_warning_colors.push(glyph.0);
                            }
                        }
                    }
                }
                let from = outlines.partition_point(|r| r.end <= ls);
                // the current match is filled instead
                let selected =
                    |r: &Range<usize>| sels.binary_search_by(|s| (s.start(), s.end()).cmp(&(r.start, r.end))).is_ok();
                for r in outlines[from..]
                    .iter()
                    .take_while(|r| r.start <= le)
                    .filter(|r| row.last || r.start < le)
                    .filter(|r| editor.results || !selected(r))
                {
                    let bounds = span(x_at(r.start.max(ls)), x_at(r.end.min(le)));
                    p.outlines.push(outline(bounds, c(theme::FIND_OUTLINE), BorderStyle::Solid));
                }
                // the number goes on a line's first row only
                let number = (!single && show_gutter && row.first && !editor.results).then(|| {
                    let number = match editor.view.line_number_mode {
                        crate::config::settings::LineNumberMode::Relative if line != primary_line => {
                            line.abs_diff(primary_line)
                        }
                        _ => line + 1,
                    };
                    #[cfg(test)]
                    drawn.numbers.push((r, number));
                    let color = if on_cursor_line { c(theme::GUTTER_FG_CURRENT) } else { c(theme::GUTTER_FG) };
                    let shaped = shape(number.to_string().into(), color);
                    #[cfg(test)]
                    drawn.number_labels.push((r, shaped.text.clone()));
                    shaped
                });
                cut_off |= row_left + shaped.width > p.text_mask.right();
                rows.push(Some((row.x0, shaped.clone())));
                p.lines.push((y, row_left, shaped, number));
            }
            if cut_off && !single && wrap.is_none() {
                let w = px(theme::OVERFLOW_SHADOW_W);
                let edge = p.text_mask.right();
                p.shadow = Some(fill(
                    Bounds::from_corners(point(edge - w, bounds.top()), point(edge, bounds.bottom())),
                    linear_gradient(
                        90.,
                        linear_color_stop(theme::ca(0x00000000), 0.),
                        linear_color_stop(theme::ca(theme::OVERFLOW_SHADOW), 1.),
                    ),
                ));
            }
            sel_rects.sort_by_key(|(i, _)| *i);
            for one in sel_rects.chunk_by(|a, b| a.0 == b.0) {
                let rects: Vec<SelRect> = one.iter().map(|(_, r)| *r).collect();
                selection_border(&rects, lh_f, c(theme::SELECTION_BORDER), &mut p.below);
            }
            let mut minimap = None;
            if !single && editor.view.minimap {
                let mm_left = bounds.right() - mm_w;
                let mm_bounds = Bounds::from_corners(point(mm_left, bounds.top()), bounds.bottom_right());
                p.minimap.push(fill(mm_bounds, editor.background()));
                p.minimap_bounds = Some(mm_bounds);
                let fit = (height / mm_lh) as usize;
                let max_scroll = (total - height as f64).max(0.);
                let mm_first = if count > fit && max_scroll > 0. {
                    (scroll_top / max_scroll * (count - fit) as f64) as usize
                } else {
                    0
                };
                let max_x = bounds.right();
                let mm_cols = (f32::from(mm_w) / mm_cw) as usize + 1;
                for r in mm_first..(mm_first + fit + 1).min(count) {
                    #[cfg(test)]
                    {
                        drawn.minimap_rows += 1;
                    }
                    let y = bounds.top() + px((r - mm_first) as f32 * mm_lh);
                    // a filler row is a gap in the minimap too
                    let Some(mut row) = row(state.doc, &folded, r) else {
                        continue;
                    };
                    // only what fits across the minimap
                    row.pieces = cut_row(&row.pieces, 0, mm_cols).0;
                    let rt = RowText::new(&state.doc.rope, &row.pieces);
                    let mut runs = rt.runs(&state.doc.rope, style.color, run, &mut cached, |line| {
                        line_colors(&mut editor.highlighter, &state.doc.rope, line, false, &mut unready)
                    });
                    let (text, cols) = expand_tabs_at(&rt.text, state.doc.tab_size, row.x0);
                    if let Cow::Owned(_) = text {
                        expand_runs(&rt.text, &cols, &mut runs);
                    }
                    let runs = runs.into_iter().map(|r| (r.len, r.color));
                    let (mut byte, mut col) = (0, row.x0);
                    for (len, mut color) in runs {
                        // Sublime's tiny glyphs cover about a third of their cell
                        color.a *= 0.35;
                        let seg = &text[byte..byte + len];
                        byte += len;
                        // every run of non-blank characters becomes one bar
                        let mut start = None;
                        for (i, ch) in seg.chars().chain(std::iter::once(' ')).enumerate() {
                            match (ch.is_whitespace(), start) {
                                (false, None) => start = Some(i),
                                (true, Some(s0)) => {
                                    let x0 = mm_left + px((col + s0) as f32 * mm_cw);
                                    let x1 = (mm_left + px((col + i) as f32 * mm_cw)).min(max_x);
                                    if x1 > x0 {
                                        p.minimap.push(fill(
                                            Bounds::from_corners(point(x0, y), point(x1, y + px(mm_lh * 0.75))),
                                            color,
                                        ));
                                    }
                                    start = None;
                                }
                                _ => {}
                            }
                        }
                        col += seg.chars().count();
                    }
                }
                // Sublime boxes the visible rows while the mouse is over the minimap
                if editor.hover.is_some_and(|h| mm_bounds.contains(&h)) {
                    let top = bounds.top() + px((first as f32 - mm_first as f32) * mm_lh);
                    let h = px(height / lh_f * mm_lh);
                    p.minimap.push(fill(
                        Bounds::new(point(mm_left, top), size(mm_w, h)),
                        theme::ca(theme::MINIMAP_VIEWPORT),
                    ));
                }
                minimap = Some((mm_left, mm_first));
            }
            // macOS overlay scroll bar: a puck while scrolling and for a moment after
            if !single && total > height as f64 && editor.scroll_bar.is_some() {
                let thumb_h = (height * height / total as f32).max(theme::SCROLL_PUCK_MIN);
                let share = scroll_top / (total - height as f64).max(1.);
                let thumb_y = share as f32 * (height - thumb_h);
                let w = px(theme::SCROLL_PUCK_W);
                let at = point(bounds.right() - px(theme::SCROLL_PUCK_RIGHT) - w, bounds.top() + px(thumb_y));
                let puck = Bounds::new(at, size(w, px(thumb_h)));
                p.scrollbar = Some(rounded(puck, w / 2., theme::ca(theme::SCROLLBAR)));
            }
            let layout = Layout {
                bounds,
                scroll_top,
                text_left,
                text_width,
                space_w,
                font: font.clone(),
                font_size,
                first_row: first,
                rows,
                minimap,
                text_area: p.text_mask,
                #[cfg(test)]
                drawn,
            };
            #[cfg(test)]
            let mut layout = layout;
            if !single && editor.view.cursor_column_highlight {
                let columns =
                    cursor_columns(state.doc, &state.view.selections, &folded, &layout, editor.scroll_left, |text| {
                        shape(text, style.color)
                    });
                #[cfg(test)]
                {
                    layout.drawn.cursor_column_shapes = columns.shapes;
                    layout.drawn.cursor_column_chars = columns.chars;
                }
                for (i, _) in layout.rows.iter().enumerate().filter(|(_, row)| row.is_some()) {
                    let r = first + i;
                    let y = bounds.top() + px((r as f64 * lh64 - scroll_top) as f32);
                    for &x in &columns.xs {
                        let band = Bounds::new(point(x, y), size(space_w, lh)).intersect(&p.text_mask);
                        let color = tint(0.045);
                        p.cursor_columns.push(fill(band, color));
                        #[cfg(test)]
                        layout.drawn.cursor_columns.push((r, band, color));
                    }
                }
            }
            // lines drawn as an edit above left them get parsed again, behind the frame
            let unready = match (unready, editor.highlighter.provisional() && !single) {
                (None, true) => {
                    let lines = |r: usize| line_near(state.doc, &folded, r);
                    Some((lines(first), lines(last.saturating_sub(1))))
                }
                (u, _) => u,
            };
            match unready {
                // the walk over unchanged lines goes on next frame, at no job's cost
                Some(_) if editor.highlighter.walking() => window.request_animation_frame(),
                Some((from, to)) => editor.highlight_in_background(from, to, cx),
                None => {}
            }
            // the completion list waited for its line's colours, and this frame made them
            editor.spelling_in_background(spell_visible, cx);
            if editor.completion_coloured(cx) {
                cx.defer_in(window, |_, _, cx| cx.notify());
            }
            // a side-by-side view tells its pair what the frame changed: its rows (the pair
            // places the filler rows again) and a scroll that kept the caret in view
            if editor.aligned {
                let key = editor.rows_key_now(cx);
                if editor.rows_key != Some(key) {
                    editor.rows_key = Some(key);
                    cx.emit(EditorEvent::RowsChanged);
                }
                if follow && (editor.scroll_top, editor.scroll_left) != editor.synced_scroll {
                    editor.scroll_synced();
                    cx.emit(EditorEvent::Scrolled);
                }
            }
            editor.layout = Some(layout);
            p
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        p: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.editor.clone()), cx);
        if !p.change_hits.is_empty() {
            let editor = self.editor.clone();
            let hits = std::mem::take(&mut p.change_hits);
            let version = p.changes_version;
            let hitbox = p.hitbox.clone();
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                if !left_press(e, phase, &hitbox, window) {
                    return;
                }
                let Some((_, index, painted)) = hits.iter().find(|(hit, _, _)| hit.contains(&e.position)) else {
                    return;
                };
                let current = editor.read(cx);
                if !current.view.gutter
                    || current.doc(cx).version != version
                    || current.changes_version != version
                    || current.changes.get(*index) != Some(painted)
                {
                    return;
                }
                cx.stop_propagation();
                editor.update(cx, |editor, cx| {
                    editor.focus(window);
                    cx.emit(EditorEvent::ShowChange(*index));
                });
            });
        }
        if !p.fold_hits.is_empty() {
            let editor = self.editor.clone();
            let hits = std::mem::take(&mut p.fold_hits);
            let version = p.changes_version;
            let hitbox = p.hitbox.clone();
            // a fold button or marker toggles its fold, and the press goes no further: no caret
            // moves, no selection starts
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                if !left_press(e, phase, &hitbox, window) {
                    return;
                }
                let Some(&(_, hit)) = hits.iter().find(|(hit, _)| hit.contains(&e.position)) else {
                    return;
                };
                // the lines it drew are another text's
                if editor.read(cx).doc(cx).version != version {
                    return;
                }
                cx.stop_propagation();
                editor.update(cx, |editor, cx| {
                    editor.focus(window);
                    match hit {
                        FoldHit::Toggle(line) => editor.toggle_fold_at(&ToggleFoldAt { line }, window, cx),
                        FoldHit::Open(at) => editor.open_fold(&OpenFold { at }, window, cx),
                    }
                });
            });
        }
        // where the pointer is over the view, for what hovering shows (the minimap's view box,
        // the fold buttons): a new frame only when that changes
        {
            let editor = self.editor.clone();
            let hitbox = p.hitbox.clone();
            let (minimap, gutter) = (p.minimap_bounds, p.gutter_bounds);
            let buttons: Vec<Bounds<Pixels>> = p.fold_buttons.iter().map(|b| b.0).collect();
            let shows = move |at: Option<Point<Pixels>>| {
                let over = |b: &Bounds<Pixels>| at.is_some_and(|at| b.contains(&at));
                let button = buttons.iter().position(over);
                (minimap.as_ref().is_some_and(over), over(&gutter), button)
            };
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let at = hitbox.is_hovered(window).then_some(e.position);
                let was = editor.read(cx).hover;
                if was == at {
                    return;
                }
                let repaint = shows(was) != shows(at);
                editor.update(cx, |ed, cx| {
                    ed.hover = at;
                    ed.hover_definition(at, window, cx);
                    if repaint {
                        cx.notify();
                    }
                });
            });
        }
        // a drag keeps selecting while the mouse is past the view's edges, where the div's
        // own move listener does not hear it
        if self.editor.read(cx).drag.is_some() {
            let editor = self.editor.clone();
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble {
                    editor.update(cx, |ed, cx| ed.on_mouse_move(e, window, cx));
                }
            });
        }
        if crate::app::perf::enabled() {
            window.on_mouse_event(|_: &ScrollWheelEvent, phase, _, _| {
                if phase == DispatchPhase::Capture {
                    crate::app::perf::input();
                }
            });
        }
        let lh = px(self.editor.read(cx).line_height());
        let focused = focus_handle.is_focused(window);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            p.gutter.drain(..).for_each(|q| window.paint_quad(q));
            for &(at, folded, color) in &p.fold_buttons {
                // Sublime's 9 × 8 ▾ in the middle of its place, turned to ▸ when folded
                let c = at.center();
                let (a, b, tip) = match folded {
                    false => (
                        point(c.x - px(4.5), c.y - px(4.)),
                        point(c.x + px(4.5), c.y - px(4.)),
                        point(c.x, c.y + px(4.)),
                    ),
                    true => (
                        point(c.x - px(4.), c.y - px(4.5)),
                        point(c.x - px(4.), c.y + px(4.5)),
                        point(c.x + px(4.), c.y),
                    ),
                };
                let mut path = Path::new(a);
                path.line_to(b);
                path.line_to(tip);
                window.paint_path(path, color);
            }
            for (y, _, _, number) in &p.lines {
                if let Some(n) = number {
                    n.paint(point(p.gutter_right - n.width, *y), lh, window, cx).ok();
                }
            }
            let text_mask = ContentMask { bounds: p.text_mask };
            window.with_content_mask(Some(text_mask), |window| {
                p.cursor_columns.drain(..).for_each(|q| window.paint_quad(q));
                p.below.drain(..).for_each(|q| window.paint_quad(q));
                for (y, x, shaped, _) in &p.lines {
                    shaped.paint(point(*x, *y), lh, window, cx).ok();
                }
                for (glyph, at) in &p.marks {
                    glyph.paint(*at, lh, window, cx).ok();
                }
                for (message, at) in &p.inline_diagnostics {
                    message.paint(*at, lh, window, cx).ok();
                }
                p.outlines.drain(..).for_each(|q| window.paint_quad(q));
                if focused {
                    p.carets.drain(..).for_each(|q| window.paint_quad(q));
                }
                if let Some(q) = p.shadow.take() {
                    window.paint_quad(q);
                }
            });
            p.minimap.drain(..).for_each(|q| window.paint_quad(q));
            if let Some(q) = p.scrollbar.take() {
                window.paint_quad(q);
            }
        });
        crate::app::perf::painted();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::single_range_in_vec_init)]
    use super::*;

    #[gpui::test]
    fn passive_occurrences_paint_other_unicode_literals_without_changing_find_or_manual_styles(
        cx: &mut gpui::TestAppContext,
    ) {
        let (e, cx) = test_editor("😀 café café CAFÉ café", cx, |e, cx| {
            e.view.selection_occurrences = true;
            e.sel(cx).set_selection(2, 6);
            e.find_ranges = vec![12..16];
            let opts = crate::editing::text::FindOpts { case_sensitive: true, ..Default::default() };
            e.read_state(cx).remember_scoped_found("CAFÉ", &opts, None, vec![12..16]);
            for ranges in &mut e.state.highlights {
                *ranges = vec![7..11];
            }
        });
        e.read_with(cx, |e, cx| {
            let drawn = &e.layout.as_ref().unwrap().drawn.occurrences;
            assert_eq!(drawn.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(), [7..11, 17..21]);
            assert!(drawn.iter().all(|(_, b)| b.size.width > px(0.)));
            assert_eq!(e.text(cx), "😀 café café CAFÉ café");
            assert_eq!(e.state.selections, [Selection::new(2, 6)]);
            assert_eq!(e.find_ranges, [12..16]);
            assert!(e.state.highlights.iter().all(|ranges| *ranges == [7..11]));
            let opts = crate::editing::text::FindOpts { case_sensitive: true, ..Default::default() };
            assert_eq!(e.read_state(cx).found("CAFÉ", &opts), Some(vec![12..16]));
            assert_eq!(e.doc(cx).version, 0);
        });
    }

    #[gpui::test]
    fn passive_occurrences_paint_visible_source_across_folds_and_wraps_without_changing_geometry(
        cx: &mut gpui::TestAppContext,
    ) {
        let (e, cx) = test_editor("é😀 é😀 é😀 é😀", cx, |e, cx| {
            e.wrap_override = Some(true);
            e.wrap_width_override = Some(6);
            e.view.minimap = false;
            e.sel(cx).set_selection(0, 2);
            e.fold_chars_now(std::iter::once(3..5), cx);
        });
        let geometry = e.read_with(cx, |e, cx| (e.display_rows(cx), e.folds.clone(), e.state.selections.clone()));
        redraw(&e, cx, |e, _| e.view.selection_occurrences = true);
        e.read_with(cx, |e, cx| {
            let drawn = &e.layout.as_ref().unwrap().drawn.occurrences;
            assert_eq!(drawn.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(), [6..8, 9..11]);
            assert!(drawn.iter().all(|(_, b)| b.size.width > px(0.)));
            assert_eq!((e.display_rows(cx), e.folds.clone(), e.state.selections.clone()), geometry);
            assert_eq!(e.text(cx), "é😀 é😀 é😀 é😀");
        });
        redraw(&e, cx, |e, _| e.view.selection_occurrences = false);
        e.read_with(cx, |e, _| assert!(e.layout.as_ref().unwrap().drawn.occurrences.is_empty()));
    }

    /// The horizontal lines of a selection's border, as (x0, x1, y).
    fn across(rects: &[SelRect]) -> Vec<(f32, f32, f32)> {
        let mut out = Vec::new();
        selection_border(rects, 15., gpui::black(), &mut out);
        (out.iter())
            .map(|q| q.bounds)
            .filter(|b| b.size.height == px(1.) && b.size.width > px(1.))
            .map(|b| (f32::from(b.left()), f32::from(b.right()), f32::from(b.top())))
            .collect()
    }

    #[test]
    fn a_selection_border_closes_above_filler_rows_and_opens_below_them() {
        // rows at 0 and 15, then two filler rows, then a row at 60
        let rects = [(10., 50., 0., true, false), (0., 50., 15., false, false), (0., 30., 60., false, true)];
        let lines = across(&rects);
        assert!(lines.contains(&(0., 50., 29.)), "closed under the second row: {lines:?}");
        assert!(lines.contains(&(0., 30., 60.)), "opened over the third: {lines:?}");
        // rows that touch keep one outline, as before
        let touching = [(10., 50., 0., true, false), (0., 50., 15., false, true)];
        assert!(!across(&touching).contains(&(0., 50., 15.)));
    }

    #[gpui::test]
    fn gutter_changes_open_the_hunk_without_moving_the_caret_and_expire_after_edits(cx: &mut gpui::TestAppContext) {
        use gpui::Modifiers;
        use std::cell::RefCell;
        use std::rc::Rc;

        let (editor, cx) = cx.add_window_view(|_, cx| {
            let mut state = EditorState::from_text("new\nstable\n");
            state.set_cursor(7);
            state.view.bookmarks.push(0);
            let mut editor = Editor::new(state, false, cx);
            editor.changes = crate::integrations::changes::between(
                &ropey::Rope::from_str("old\nstable\n"),
                &doc_of(&editor.document, cx).rope,
            );
            editor
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let received = events.clone();
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
                if let EditorEvent::ShowChange(index) = event {
                    received.borrow_mut().push(*index);
                }
            })
        });
        cx.run_until_parked();
        let (marker, bookmark) = editor.read_with(cx, |editor, _| {
            let bounds = editor.layout.as_ref().unwrap().bounds;
            let y = bounds.top() + px(editor.line_height() / 2.);
            (point(bounds.left() + px(3.), y), point(bounds.left() + px(11.), y))
        });
        cx.simulate_click(marker, Modifiers::none());
        assert_eq!(*events.borrow(), [0]);
        assert_eq!(editor.read_with(cx, |editor, _| editor.state.cursor()), 7);
        cx.simulate_click(bookmark, Modifiers::none());
        assert_eq!(*events.borrow(), [0]);
        assert_eq!(editor.read_with(cx, |editor, _| editor.state.cursor()), 0);
        // an edit leaves the hunks stale: their marker takes no click, before the next paint or
        // after it
        editor.update(cx, |editor, cx| editor.edit(cx, |s| s.insert("x")));
        cx.simulate_click(marker, Modifiers::none());
        assert_eq!(*events.borrow(), [0]);
        editor.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        cx.simulate_click(marker, Modifiers::none());
        assert_eq!(*events.borrow(), [0]);
        // a new state starts its version over, which must not bring the old hunks back
        editor.update(cx, |editor, cx| editor.set_text("different\ncontent\n", cx));
        cx.run_until_parked();
        cx.simulate_click(marker, Modifiers::none());
        assert_eq!(*events.borrow(), [0]);
    }

    #[test]
    fn change_markers_distinguish_changed_lines_and_deleted_boundaries() {
        let hunk = |old_lines, new_lines| crate::integrations::changes::Hunk {
            old_chars: 0..0,
            new_chars: 0..0,
            old_lines,
            new_lines,
        };
        let hunks = [hunk(1..1, 1..3), hunk(3..4, 5..6), hunk(6..7, 7..7)];
        assert_eq!(change_marker(&hunks, 0, 10), None);
        assert_eq!(change_marker(&hunks, 1, 10), Some((0, ChangeMarker::Added)));
        assert_eq!(change_marker(&hunks, 2, 10), Some((0, ChangeMarker::Added)));
        assert_eq!(change_marker(&hunks, 3, 10), None);
        assert_eq!(change_marker(&hunks, 5, 10), Some((1, ChangeMarker::Modified)));
        assert_eq!(change_marker(&hunks, 7, 10), Some((2, ChangeMarker::Deleted(false))));
        assert_eq!(change_marker(&[hunk(3..4, 3..3)], 2, 3), Some((0, ChangeMarker::Deleted(true))));
        assert_eq!(change_marker(&[hunk(3..4, 3..3)], 3, 4), Some((0, ChangeMarker::Deleted(false))));
        assert_eq!(change_marker(&[hunk(0..1, 0..0)], 0, 1), Some((0, ChangeMarker::Deleted(false))));
    }

    fn runs(lens: &[usize]) -> Vec<TextRun> {
        let font = gpui::font(theme::EDITOR_FONT);
        (lens.iter()).map(|&n| text_run(n, &font, c(theme::EDITOR_FG))).collect()
    }

    #[gpui::test]
    #[ignore = "release performance gate"]
    fn bench_a_frame_of_two_thousand_selections_on_one_row(cx: &mut TestAppContext) {
        let (e, cx) = test_editor(&"ab".repeat(2_000), cx, |_, _| {});
        e.update(cx, |e, cx| {
            let sels: Vec<Selection> = (0..2_000).map(|i| Selection::new(2 * i, 2 * i + 1)).collect();
            e.sel(cx).set_selections(sels);
        });
        redraw(&e, cx, |_, _| {});
        let start = std::time::Instant::now();
        for _ in 0..10 {
            redraw(&e, cx, |_, _| {});
        }
        let frame = start.elapsed() / 10;
        eprintln!("a frame of 2,000 selections on one 4,000-char row: {frame:?}");
        assert!(frame < std::time::Duration::from_millis(8));
    }

    #[test]
    fn recolor_each_colours_every_byte_as_recolor_once_a_range_does() {
        let (red, blue) = (c(0xff0000), c(0x0000ff));
        let mut seed = 7u64;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % n
        };
        let bytes =
            |runs: &[TextRun]| -> Vec<Hsla> { runs.iter().flat_map(|r| std::iter::repeat_n(r.color, r.len)).collect() };
        for _ in 0..2_000 {
            let lens: Vec<usize> = (0..1 + next(5)).map(|_| 1 + next(6)).collect();
            let total: usize = lens.iter().sum();
            // sorted and disjoint, some empty, some touching, some across runs
            let (mut ranges, mut at) = (Vec::new(), 0);
            while ranges.len() < 6 {
                let a = at + next(3);
                if a > total {
                    break;
                }
                let b = (a + next(6)).min(total);
                ranges.push((a, b, if next(2) == 0 { red } else { blue }));
                at = b;
            }
            let want = ranges.iter().fold(runs(&lens), |r, &(a, b, color)| recolor(r, a, b, color));
            let got = recolor_each(runs(&lens), &ranges);
            assert_eq!(
                bytes(&got),
                bytes(&want),
                "{lens:?} {:?}",
                ranges.iter().map(|r| (r.0, r.1)).collect::<Vec<_>>()
            );
            assert!(got.iter().all(|r| r.len > 0));
        }
    }

    #[test]
    fn a_rows_runs_are_cut_restyled_and_widened_by_bytes() {
        use crate::languages::highlight::Look;
        let red = c(0xff0000);
        // the colours of bytes 3..9 of a line coloured 0..5 and 5..12
        let coloured = [(5, red, Look::default()), (7, red, Look::default())];
        let cut: Vec<usize> = slice_runs(&coloured, 3, 9).iter().map(|r| r.0).collect();
        assert_eq!(cut, [2, 4]);
        // a recolour splits the runs at its edges and changes only what is inside
        let out = recolor(runs(&[4, 4]), 2, 6, red);
        let lens: Vec<usize> = out.iter().map(|r| r.len).collect();
        assert_eq!(lens, [2, 2, 2, 2]);
        assert_eq!(out.iter().filter(|r| r.color == red).count(), 2);
        let underlined = restyle(runs(&[8]), 1, 3, |r| r.underline = Some(Default::default()));
        assert_eq!(underlined.iter().map(|r| r.underline.is_some()).collect::<Vec<_>>(), [false, true, false]);
        // a tab drawn four columns wide: its run grows by the spaces it became
        let text = "a\tb";
        let (_, cols) = crate::editing::display::expand_tabs(text, 4);
        let mut r = runs(&[1, 1, 1]);
        expand_runs(text, &cols, &mut r);
        assert_eq!(r.iter().map(|r| r.len).collect::<Vec<_>>(), [1, 3, 1]);
    }

    #[test]
    fn rendered_spelling_wave_clips_to_unicode_rows_and_preserves_other_styles() {
        let rope = ropey::Rope::from_str("😀 café mispelled end");
        let row = 5..18;
        let mut input = runs(&[rope.slice(row.clone()).len_bytes()]);
        input[0].background_color = Some(c(0x123456));
        let text = RowText::new(&rope, &[Piece::Text(row.clone())]);
        let output = spell_underlines(input, &rope, &text, row.clone(), [7..16]);
        let waves: Vec<_> = output.iter().filter_map(|r| r.underline.map(|u| (r.len, u))).collect();
        assert_eq!(waves.len(), 1);
        assert_eq!(waves[0].0, "mispelled".len());
        assert!(waves[0].1.wavy);
        assert_eq!(waves[0].1.color, Some(c(0xec5f66)));
        assert!(output.iter().all(|r| r.background_color == Some(c(0x123456))));
        assert_eq!(output.iter().map(|r| r.len).sum::<usize>(), rope.slice(row).len_bytes());
    }

    #[test]
    fn a_caret_far_into_a_long_line_of_wide_chars_lands_in_view() {
        let (sw, w, margin) = (7., 700., 40.);
        // CJK draws 2 columns a char, tabs more; plain text 1
        for k in [2., 1., 4., 0.5] {
            let width_from = |cc: usize| move |a: usize| (cc - a) as f32 * k * sw;
            for (cc, left) in [(5000, 0.), (20_000, 0.), (1_000_000, 0.), (5000, 1e6), (300, 0.)] {
                let left = long_follow(cc, left, w, margin, sw, width_from(cc));
                // where `clip` draws the caret for that scroll
                let a = clip_start(left, sw).min(cc);
                let drawn = a as f32 * sw + width_from(cc)(a);
                // inside the margins, give or take the char the start falls on
                let slack = k * sw + 1.;
                assert!(
                    drawn >= left + margin - slack && drawn <= left + w - margin + slack,
                    "k {k}, caret {cc}: drawn at {drawn}, view {left}..{}",
                    left + w
                );
            }
        }
    }

    /// A row that is a whole display line, as `clip` is given it.
    fn whole(chars: Range<usize>, pieces: Vec<Piece>) -> Row {
        Row { line: 0, chars, x0: 0, first: true, last: true, pieces }
    }

    #[test]
    fn a_diagnostic_ending_at_a_row_start_stays_off_that_row() {
        // "ab\ncd": an LSP range 0:0-1:0 is chars 0..3
        assert_eq!(diagnostic_on_row(&(0..3), 0, 2), Some(0..2));
        assert_eq!(diagnostic_on_row(&(0..3), 3, 5), None);
        // one that starts there, empty or not, is on it
        assert_eq!(diagnostic_on_row(&(3..3), 3, 5), Some(3..4));
        assert_eq!(diagnostic_on_row(&(3..4), 3, 5), Some(3..4));
        assert_eq!(diagnostic_on_row(&(1..4), 3, 5), Some(3..4));
        // at a line's end, its last char
        assert_eq!(diagnostic_on_row(&(2..2), 0, 2), Some(1..2));
    }

    #[test]
    fn clip_cuts_a_long_row_around_the_view() {
        let row = whole(100..100_100, vec![Piece::Text(100..100_100)]);
        let r = clip(row, 7000., 700., 7.);
        assert_eq!(r.chars, 100 + 1000 - CLIP_MARGIN..100 + 1100 + CLIP_MARGIN);
        assert_eq!(r.x0, 1000 - CLIP_MARGIN);
        assert!(!r.last);
        let r = clip(whole(0..5000, vec![Piece::Text(0..5000)]), 0., 700., 7.);
        assert_eq!((r.chars, r.last), (0..100 + CLIP_MARGIN, false));
        // a row with a fold, cut by what it shows: a placeholder the cut starts in is whole
        let pieces = vec![Piece::Text(0..10), Piece::Fold(10..150_010), Piece::Text(150_010..200_000)];
        let row = whole(0..200_000, pieces);
        let r = clip(row, (11 + CLIP_MARGIN) as f32 * 7., 700., 7.);
        // to display char 11 + 100 + 2 margins, 13 of them before the text after the fold
        let end = 150_010 + 11 + 100 + 2 * CLIP_MARGIN - 13;
        assert_eq!((r.x0, r.chars.clone(), r.last), (10, 10..end, false));
        assert_eq!(r.pieces, [Piece::Fold(10..150_010), Piece::Text(150_010..end)]);
    }

    #[test]
    fn guide_levels_count_the_indent_and_carry_over_blank_lines() {
        let long = format!("        {}", "x ".repeat(100_000));
        let text = format!("fn a() {{\n\tif x {{\n\t\ty();\n\n\t}}\n  {long}\n}}\n");
        let mut s = EditorState::from_text(&text);
        s.doc.tab_size = 4;
        let levels: Vec<usize> = (0..7).map(|l| guide_levels(&s.doc, l)).collect();
        assert_eq!(levels, [0, 1, 2, 2, 1, 2, 0], "the blank line takes its deeper neighbour");
    }

    use gpui::{size, Entity, TestAppContext, VisualTestContext};
    use Mark::{Eol, Other, Space, Tab};

    const TRAILING: WhiteSpace = WhiteSpace { trailing: true, ..WhiteSpace::NONE };

    #[test]
    fn white_space_marks_keep_unicode_text_between_multiple_folds() {
        let rope = ropey::Rope::from_str("aXéY z");
        let row = RowText::new(
            &rope,
            &[Piece::Text(0..1), Piece::Fold(1..2), Piece::Text(2..3), Piece::Fold(3..4), Piece::Text(4..6)],
        );
        let marked = row.marked();
        assert_eq!(marked, "a………é……… z");
        assert_eq!(marks(&marked, 0..10, &[], WhiteSpace::ALL, false), [(8, Space)]);
    }

    /// Every mark of a whole line in "all".
    fn all(line: &str, has_eol: bool) -> Vec<(usize, Mark)> {
        marks(line, 0..line.chars().count(), &[], WhiteSpace::ALL, has_eol)
    }

    #[test]
    fn all_marks_every_space_tab_other_space_and_the_line_end() {
        // a ␠ b ⇥ c nbsp d
        assert_eq!(all("a b\tc\u{a0}d", true), [(1, Space), (3, Tab), (5, Other), (7, Eol)]);
        assert_eq!(all("  \t", true), [(0, Space), (1, Space), (2, Tab), (3, Eol)]);
        // a line without a break has no line end; an empty one with a break has
        assert_eq!(all("a b", false), [(1, Space)]);
        assert_eq!(all("", true), [(0, Eol)]);
        assert_eq!(all("", false), []);
        // chars are counted, not bytes
        assert_eq!(all("é ü\t日 ", true), [(1, Space), (3, Tab), (5, Space), (6, Eol)]);
        // the other Unicode spaces `white` marks, the ends and middle of each range
        let others = "\u{a0}\u{2000}\u{2005}\u{2007}\u{200a}\u{202f}\u{205f}\u{3000}";
        let got = all(others, false);
        assert_eq!(got.len(), 8);
        assert!(got.iter().all(|&(_, m)| m == Other), "{got:?}");
        // and nothing next to them: zero width, line separators, controls, punctuation
        let not = "\u{1fff}\u{200b}\u{2028}\u{2029}\u{feff}\u{85}\u{1680}\u{2fff}\u{3001}\r\u{b}x";
        assert_eq!(all(not, false), []);
    }

    #[test]
    fn selection_marks_inside_selections_and_trailing_after_the_last_visible_char() {
        // 0 a, 1 ␠, 2 b, 3 ␠, 4 ␠, 5 c, 6 ⇥, 7 ␠, 8 d, 9 ␠, 10 ␠
        let line = "a b  c\t d  ";
        let n = line.chars().count();
        let selection = WhiteSpace::SELECTION;
        let trailing = TRAILING;
        let run = |ws, sels: &[Range<usize>]| marks(line, 0..n, sels, ws, true);
        // selection: the white space inside non-empty selections, and no line end
        assert_eq!(run(selection, &[]), []);
        assert_eq!(run(selection, &[3..5]), [(3, Space), (4, Space)]);
        assert_eq!(run(selection, &[2..4, 6..9]), [(3, Space), (6, Tab), (7, Space)]);
        assert_eq!(run(selection, &[5..5]), [], "a caret marks nothing");
        assert_eq!(run(selection, &[8..99]), [(9, Space), (10, Space)], "a selection that takes the line break too");
        // trailing: after the last char that is not white space
        assert_eq!(run(trailing, &[0..n]), [(9, Space), (10, Space)]);
        assert_eq!(
            marks("   ", 0..3, &[], trailing, true),
            [(0, Space), (1, Space), (2, Space)],
            "a line of white space is all trailing"
        );
        assert_eq!(marks("a\t \u{a0}", 0..4, &[], trailing, true), [(1, Tab), (2, Space), (3, Other)]);
        assert_eq!(marks("a b", 0..3, &[], trailing, true), []);
        assert_eq!(marks("", 0..0, &[], trailing, true), []);
        // the modes add up
        let both = WhiteSpace { selection: true, ..trailing };
        assert_eq!(run(both, &[2..4]), [(3, Space), (9, Space), (10, Space)]);
        assert_eq!(run(WhiteSpace::NONE, &[0..n]), []);
        assert_eq!(run(WhiteSpace::ALL, &[]).last(), Some(&(n, Eol)));
        assert_eq!(run(WhiteSpace::ALL, &[]).len(), 8);
    }

    #[test]
    fn a_wrapped_rows_marks_are_the_lines_marks_within_it() {
        // 0 a, 1 b, 2 ␠, 3 c, 4 d, 5 ␠, 6 ␠, 7 e, 8 f, 9 ␠, 10 g, 11 h, 12 ␠
        let line = "ab cd  ef gh ";
        // rows of 6, 4 and 3 chars: the break falls between the two spaces of the double
        let rows = [0..6, 6..10, 10..13];
        let whole = marks(line, 0..13, &[], WhiteSpace::ALL, true);
        let per_row: Vec<Vec<(usize, Mark)>> =
            rows.iter().map(|seg| marks(line, seg.clone(), &[], WhiteSpace::ALL, true)).collect();
        assert_eq!(
            per_row,
            [vec![(2, Space), (5, Space)], vec![(6, Space), (9, Space)], vec![(12, Space), (13, Eol)],],
            "each mark is on the row that draws it, the line end on the last"
        );
        assert_eq!(per_row.concat(), whole);
        // a selection across a break is cut into its rows
        let sel = WhiteSpace::SELECTION;
        assert_eq!(marks(line, 0..6, &[3..8], sel, true), [(5, Space)]);
        assert_eq!(marks(line, 6..10, &[3..8], sel, true), [(6, Space)]);
        // trailing white space that runs over several rows is trailing on each
        let trailing = TRAILING;
        let line = "ab      ";
        assert_eq!(marks(line, 0..4, &[], trailing, true), [(2, Space), (3, Space)]);
        assert_eq!(marks(line, 4..8, &[], trailing, true), [(4, Space), (5, Space), (6, Space), (7, Space)]);
        // but white space at a break with text after it is not
        let line = "ab    cd  ";
        assert_eq!(marks(line, 0..6, &[], trailing, true), []);
        assert_eq!(marks(line, 6..10, &[], trailing, true), [(8, Space), (9, Space)]);
        // a row cut out of a line of multi-byte chars counts chars
        assert_eq!(marks("ééé ü ", 3..6, &[], WhiteSpace::ALL, true), [(3, Space), (5, Space), (6, Eol)]);
    }

    #[test]
    fn selected_in_gives_the_non_empty_selections_that_touch_a_span_from_its_start() {
        let sels = [
            Selection::caret(1),
            Selection::new(4, 8),
            Selection::new(12, 10),
            Selection::caret(14),
            Selection::new(20, 30),
        ];
        assert_eq!(selected_in(&sels, 6, 12), [0..2, 4..6]);
        assert!(selected_in(&sels, 8, 10).is_empty(), "touching is not inside");
        assert_eq!(selected_in(&sels, 0, 100), [4..8, 10..12, 20..30]);
        assert_eq!(selected_in(&sels, 25, 26), [0..5]);
        assert!(selected_in(&[], 0, 5).is_empty());
    }

    /// An editor alone in a 600 × 300 window, drawn once, wrapped at `wrap` columns or not at
    /// all. gpui's test text system lays text out monospace.
    fn white_editor<'a>(
        text: &str,
        white: WhiteSpace,
        wrap: Option<usize>,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Editor>, &'a mut VisualTestContext) {
        test_editor(text, cx, move |e, _| {
            e.view.white_space = white;
            e.wrap_override = Some(wrap.is_some());
            e.wrap_width_override = wrap;
        })
    }

    /// Change what the view marks and draw again.
    fn redraw(
        e: &Entity<Editor>,
        cx: &mut VisualTestContext,
        change: impl FnOnce(&mut Editor, &mut Context<Editor>) + 'static,
    ) {
        e.update(cx, |e, cx| {
            change(e, cx);
            cx.notify();
        });
        cx.run_until_parked();
    }

    /// `v` in whole `unit`s: on the grid or the frame is wrong. The test text system adds up a
    /// space's width char by char, and f32 drifts.
    fn grid(v: Pixels, unit: Pixels) -> usize {
        let n = v / unit;
        assert!((n - n.round()).abs() < 0.1, "{n} is off the grid");
        n.round() as usize
    }

    /// The last frame's marks as (char, mark, row, column, columns wide), the columns counted
    /// from the text's left edge.
    fn marked(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<(usize, Mark, usize, usize, usize)> {
        e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            (l.drawn.marks.iter())
                .map(|&(at, mark, cell, _)| {
                    let row = grid(cell.top() - l.bounds.top(), px(e.line_height()));
                    let left = grid(cell.left() - l.text_left, l.space_w);
                    (at, mark, row, left, grid(cell.size.width, l.space_w))
                })
                .collect()
        })
    }

    /// The last frame's tab lines as (row, first column, last column, pixels down the row).
    fn tab_lines(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<(usize, usize, usize, f32)> {
        e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            let lh = px(e.line_height());
            (l.drawn.tab_lines.iter())
                .map(|quad| {
                    let b = quad.bounds;
                    let down = b.top() - l.bounds.top();
                    let row = (down / lh).floor();
                    assert_eq!(f32::from(b.size.height), 1., "thin");
                    let col = |x: Pixels| grid(x - l.text_left, l.space_w);
                    (row as usize, col(b.left()), col(b.right()), f32::from(down - lh * row))
                })
                .collect()
        })
    }

    #[gpui::test]
    fn marks_sit_in_the_cells_the_shaped_line_uses(cx: &mut TestAppContext) {
        // the tabs at columns 3, 0 and 2 all reach a stop of 4; the last line has no break
        let text = "a b\tc\u{a0}d  \n\tx\nab\tx\nlast ";
        let (e, cx) = white_editor(text, WhiteSpace::ALL, None, cx);
        assert_eq!(
            marked(&e, cx),
            [
                (1, Space, 0, 1, 1),
                (3, Tab, 0, 3, 1),
                (5, Other, 0, 5, 1),
                (7, Space, 0, 7, 1),
                (8, Space, 0, 8, 1),
                (9, Eol, 0, 9, 1),
                (10, Tab, 1, 0, 4),
                (12, Eol, 1, 5, 1),
                (15, Tab, 2, 2, 2),
                (17, Eol, 2, 5, 1),
                (22, Space, 3, 4, 1),
            ]
        );
        // a line from the arrow to the stop, level with its shaft: the tab one column wide has
        // none
        let down = (e.read_with(cx, |e, _| e.line_height()) * 0.56).floor();
        assert_eq!(
            tab_lines(&e, cx),
            [(1, 1, 4, down), (2, 3, 4, down)],
            "the tab at column 3 reaches its stop in the arrow's cell"
        );
        // wider tab stops are the cells the text is drawn at, so the marks follow them
        redraw(&e, cx, |e, cx| e.update_doc(cx, |d| d.tab_size = 8));
        let tabs: Vec<_> = (marked(&e, cx).into_iter()).filter(|m| m.1 == Tab).map(|m| (m.0, m.3, m.4)).collect();
        assert_eq!(tabs, [(3, 3, 5), (10, 0, 8), (15, 2, 6)]);
    }

    #[gpui::test]
    fn selection_and_trailing_mark_only_what_they_name(cx: &mut TestAppContext) {
        let text = "a b\tc\u{a0}d  \n\tx\nab\tx\nlast ";
        // "selection", the default, draws nothing until something is selected
        let (e, cx) = white_editor(text, WhiteSpace::SELECTION, None, cx);
        assert_eq!(marked(&e, cx), []);
        redraw(&e, cx, |e, cx| e.sel(cx).set_selection(2, 11));
        let at = |m: Vec<(usize, Mark, usize, usize, usize)>| -> Vec<usize> { m.iter().map(|m| m.0).collect() };
        assert_eq!(at(marked(&e, cx)), [3, 5, 7, 8, 10], "no line end, even inside");
        // a backward selection and a second one mark alike
        redraw(&e, cx, |e, cx| e.sel(cx).set_selections(vec![Selection::new(2, 1), Selection::new(23, 21)]));
        assert_eq!(at(marked(&e, cx)), [1, 22]);
        // "trailing" marks after the last char of each line, selection or not
        redraw(&e, cx, |e, _| e.view.white_space = TRAILING);
        assert_eq!(marked(&e, cx), [(7, Space, 0, 7, 1), (8, Space, 0, 8, 1), (22, Space, 3, 4, 1)]);
        // and "none" nothing
        redraw(&e, cx, |e, _| e.view.white_space = WhiteSpace::NONE);
        assert_eq!(marked(&e, cx), []);
    }

    #[gpui::test]
    fn wrapped_rows_draw_their_own_marks_at_their_indent(cx: &mut TestAppContext) {
        // rows "    ab cd  " | "ef gh " | "ij kl" at 12 columns; the last two start at column 4
        let (e, cx) = white_editor("    ab cd  ef gh ij kl\n", WhiteSpace::ALL, Some(12), cx);
        assert_eq!(
            marked(&e, cx),
            [
                (0, Space, 0, 0, 1),
                (1, Space, 0, 1, 1),
                (2, Space, 0, 2, 1),
                (3, Space, 0, 3, 1),
                (6, Space, 0, 6, 1),
                (9, Space, 0, 9, 1),
                (10, Space, 0, 10, 1),
                (13, Space, 1, 6, 1),
                (16, Space, 1, 9, 1),
                (19, Space, 2, 6, 1),
                (22, Eol, 2, 9, 1),
            ]
        );
        // trailing white space is where the line ends, not where a row does: rows
        //   "ab cd  " | "ef gh " | "ij kl   "   three trailing spaces
        //   "ab      " | "        " | "      "   twenty, over three rows
        //   "ab    " | "cd  "                    four with "cd" after them, then two
        let text = "ab cd  ef gh ij kl   \nab                    \nab    cd  ";
        let (e, cx) = white_editor(text, WhiteSpace::NONE, Some(8), cx);
        redraw(&e, cx, |e, _| e.view.white_space = TRAILING);
        // `n` spaces from char `at`, the first at (row, col)
        let spaces =
            |at: usize, row: usize, col: usize, n: usize| (0..n).map(move |k| (at + k, Space, row, col + k, 1));
        let want: Vec<_> = spaces(18, 2, 5, 3)
            .chain(spaces(24, 3, 2, 6))
            .chain(spaces(30, 4, 0, 8))
            .chain(spaces(38, 5, 0, 6))
            .chain(spaces(53, 7, 2, 2))
            .collect();
        assert_eq!(marked(&e, cx), want);
    }

    #[gpui::test]
    fn a_row_far_from_its_lines_end_does_not_mark_white_space_as_trailing(cx: &mut TestAppContext) {
        // 400 spaces between "ab" and "x": none of them is trailing, though the rows that
        // cannot see the "x" (the first ones, over 256 chars from it) cannot tell
        let trailing = TRAILING;
        let text = format!("ab{}x", " ".repeat(400));
        let (e, cx) = white_editor(&text, trailing, Some(80), cx);
        assert_eq!(marked(&e, cx), []);
        // with nothing after the spaces, the rows that see the end of the line mark theirs: the
        // first row to end within 256 chars of it on, and the earlier ones none
        let text = format!("ab{}", " ".repeat(400));
        let (e, cx) = white_editor(&text, trailing, Some(80), cx);
        let rows: Vec<Range<usize>> = e.read_with(cx, |e, cx| {
            let w = e.wrap.as_ref().unwrap();
            (0..w.rows(e.doc(cx))).map(|r| w.row_bounds(e.doc(cx), DisplayRow(r)).unwrap()).collect()
        });
        let sees = rows.iter().find(|r| r.end + TRAIL_LOOK >= 402).unwrap();
        assert!(sees.start > 0, "the rows are {rows:?}");
        let at: Vec<usize> = marked(&e, cx).iter().map(|m| m.0).collect();
        assert_eq!(at, (sees.start..402).collect::<Vec<_>>());
    }

    #[gpui::test]
    fn a_single_line_field_draws_no_marks(cx: &mut TestAppContext) {
        let (e, cx) = cx.add_window_view(|_, cx| {
            let mut e = Editor::new(EditorState::from_text("a  b\t"), true, cx);
            e.view.white_space = WhiteSpace::ALL;
            e.sel(cx).set_selection(0, 5);
            e
        });
        cx.simulate_resize(size(px(300.), px(30.)));
        cx.run_until_parked();
        assert_eq!(marked(&e, cx), []);
    }

    #[gpui::test]
    fn hidden_lines_draw_no_marks_and_a_folded_header_its_own(cx: &mut TestAppContext) {
        // lines 1 and 2 fold under the header, whose "…" marker its line end has to clear
        let text = "fn a() { \n    x \n    y\n}\nlast \n";
        let (e, cx) = white_editor(text, WhiteSpace::ALL, None, cx);
        redraw(&e, cx, |e, cx| e.fold_lines_now(0, 2, cx));
        // (char, mark, row) of each mark, the header's line end and its marker
        let frame = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.read_with(cx, |e, _| {
                let l = e.layout.as_ref().unwrap();
                let row = |b: Bounds<Pixels>| ((b.top() - l.bounds.top()) / px(e.line_height())).round();
                let marks: Vec<_> =
                    (l.drawn.marks.iter()).map(|&(at, mark, cell, _)| (at, mark, row(cell) as usize)).collect();
                let eol = (l.drawn.marks.iter()).find(|m| m.1 == Eol).map(|m| m.2);
                (marks, eol, l.drawn.markers.clone())
            })
        };
        let (m, eol, markers) = frame(&e, cx);
        assert_eq!(
            m,
            [(2, Space, 0), (6, Space, 0), (8, Space, 0), (22, Eol, 0), (24, Eol, 1), (29, Space, 2), (30, Eol, 2),],
            "nothing from the hidden lines, whose text is at 10 to 21; the header's line end is \
             the break after the fold, at 22"
        );
        // the header's line end starts where its marker ends
        let [(0, marker)] = markers[..] else { panic!("one marker: {markers:?}") };
        assert_eq!(eol.unwrap().left(), marker.right());
        // a selection across the fold marks what is shown of it
        redraw(&e, cx, |e, cx| {
            e.view.white_space = WhiteSpace::SELECTION;
            e.sel(cx).set_selection(8, 30);
        });
        assert_eq!(frame(&e, cx).0, [(8, Space, 0), (29, Space, 2)], "the header's last space and the shown line's");
        // a header that wraps: each of its rows has its marks, and the last its line end. The
        // placeholder is laid out with the text: "{" and its three columns go to a third row
        let text = "ab cd ef gh {\n  x \n  y\nz ";
        let (e, cx) = white_editor(text, WhiteSpace::ALL, Some(8), cx);
        redraw(&e, cx, |e, cx| e.fold_lines_now(0, 2, cx));
        let (m, eol, markers) = frame(&e, cx);
        assert_eq!(m, [(2, Space, 0), (5, Space, 0), (8, Space, 1), (11, Space, 1), (22, Eol, 2), (24, Space, 3),]);
        assert_eq!(markers.len(), 1);
        assert_eq!(eol.unwrap().left(), markers[0].1.right());
    }

    #[gpui::test]
    fn a_wrapped_display_line_marks_the_rows_after_its_fold_by_their_own_line(cx: &mut TestAppContext) {
        // the fold hides "\nb\n", from "a"'s end to the start of the long line: that line goes
        // on after the placeholder, over rows of its own
        let text = format!("a\nb\n{}\n", "word ".repeat(30));
        let (e, cx) = white_editor(&text, WhiteSpace::ALL, Some(20), cx);
        redraw(&e, cx, |e, cx| e.fold_chars_now([1..4], cx));
        let marks = e.read_with(cx, |e, _| e.layout.as_ref().unwrap().drawn.marks.clone());
        // every space of the long line is marked, on whichever row it falls
        let spaces = (marks.iter()).filter(|m| m.1 == Mark::Space && m.0 > 4).count();
        assert_eq!(spaces, 30, "{marks:?}");
    }

    #[gpui::test]
    fn a_fold_to_the_end_of_text_draws_a_line_end_only_when_a_line_break_follows_it(cx: &mut TestAppContext) {
        // lines 1 and 2 fold under the header, whose own spaces are at 2, 6 and 8
        for (end, want_eol) in [("", false), ("\n", true)] {
            let text = format!("fn a() {{ \n    x \n    y{end}");
            let (e, cx) = white_editor(&text, WhiteSpace::ALL, None, cx);
            redraw(&e, cx, |e, cx| e.fold_lines_now(0, 2, cx));
            let mut want = vec![(2, Space), (6, Space), (8, Space)];
            // the line end drawn is the break after the fold's end, at 22
            if want_eol {
                want.push((22, Eol));
            }
            // the line end after the marker is off the column grid `marked` checks
            let got: Vec<_> = colored(&e, cx).iter().map(|m| (m.0, m.1)).collect();
            assert_eq!(got, want, "{text:?}");
        }
    }

    /// Two editors side by side.
    struct Pair(Entity<Editor>, Entity<Editor>);

    impl gpui::Render for Pair {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            use gpui::prelude::*;
            let side = |e: &Entity<Editor>| gpui::div().w(px(300.)).h_full().child(e.clone());
            gpui::div().flex().size_full().child(side(&self.0)).child(side(&self.1))
        }
    }

    #[gpui::test]
    fn two_views_of_a_buffer_mark_their_own_selections(cx: &mut TestAppContext) {
        let (pair, cx) = cx.add_window_view(|_, cx| {
            let a = cx.new(|cx| {
                let mut e = Editor::new(EditorState::from_text("a  b\n"), false, cx);
                e.wrap_override = Some(false);
                e
            });
            let b = a.update(cx, |a, cx| cx.new(|cx| a.another_view(cx)));
            Pair(a, b)
        });
        cx.simulate_resize(size(px(600.), px(300.)));
        cx.run_until_parked();
        let (a, b) = pair.read_with(cx, |p, _| (p.0.clone(), p.1.clone()));
        let at = |e: &Entity<Editor>, cx: &mut VisualTestContext| -> Vec<usize> {
            marked(e, cx).iter().map(|m| m.0).collect()
        };
        assert_eq!((at(&a, cx), at(&b, cx)), (vec![], vec![]));
        // one view selects the spaces; the other, sharing the text, has nothing marked
        redraw(&a, cx, |e, cx| e.sel(cx).set_selection(0, 4));
        assert_eq!((at(&a, cx), at(&b, cx)), (vec![1, 2], vec![]));
        redraw(&b, cx, |e, cx| e.sel(cx).set_selection(2, 4));
        assert_eq!((at(&a, cx), at(&b, cx)), (vec![1, 2], vec![2]));
    }

    #[gpui::test]
    fn a_huge_line_marks_only_the_rows_on_screen(cx: &mut TestAppContext) {
        // 300,000 chars on a line, then short lines
        let text = format!("{}\nend \nmore \n", "a ".repeat(150_000));
        for wrap in [None, Some(80)] {
            let (e, cx) = white_editor(&text, WhiteSpace::ALL, wrap, cx);
            let m = marked(&e, cx);
            // the view's width and a margin of the long line, or its first rows (17 of 80
            // columns); never its 150,000 spaces
            assert!(!m.is_empty() && m.len() < 1_000, "{} marks", m.len());
            if wrap.is_none() {
                // the short lines below it are rows too
                assert!(m.iter().any(|m| m.0 >= 300_001), "the next lines");
            }
            assert!(m.iter().filter(|m| m.0 < 300_000).all(|m| m.0 < 1_400));
        }
    }

    #[gpui::test]
    fn char_xs_is_x_for_index_at_every_char_of_a_row(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text(""), false, cx));
        cx.update(|window, _| {
            let font = gpui::font(theme::EDITOR_FONT);
            // tabs, two-byte and four-byte chars, a row that starts in the line's column 3
            for text in ["a", "ab c", "\ta\tbb\t", "é\u{a0}😀 x\u{3000}y😀", "   ", "日本 語\t"] {
                let (shown, cols) = expand_tabs_at(text, 4, 3);
                let run = text_run(shown.len(), &font, c(theme::EDITOR_FG));
                let shaped =
                    window.text_system().shape_line(SharedString::from(shown.to_string()), px(12.), &[run], None);
                let want: Vec<Pixels> = cols.iter().map(|&col| shaped.x_for_index(byte_of(&shown, col))).collect();
                assert_eq!(char_xs(&shown, &cols, &shaped), want, "{text:?}");
            }
        });
    }

    #[gpui::test]
    fn marks_are_made_for_the_chars_in_view_only(cx: &mut TestAppContext) {
        // 40 rows of 3,000 chars with a space in every third
        let text = format!("{}\n", "ab ".repeat(1000)).repeat(40);
        let (e, cx) = white_editor(&text, WhiteSpace::ALL, None, cx);
        // the columns across the text area, a space's width, and the rows drawn
        let view = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.read_with(cx, |e, _| {
                let l = e.layout.as_ref().unwrap();
                let (sw, left) = (f32::from(l.space_w), e.scroll_left);
                (left / sw, (left + f32::from(l.text_width)) / sw)
            })
        };
        for scroll in [0., 1000., 15_000.] {
            redraw(&e, cx, move |e, _| e.scroll_left = scroll);
            let (from, to) = view(&e, cx);
            let m = marked(&e, cx);
            let rows = m.iter().map(|m| m.2).max().unwrap() + 1;
            // a screen's width of marks a row, in the columns in view (one in three is a space)
            let per_row = (to - from) as usize / 3 + 3;
            assert!(m.len() <= rows * per_row, "{} marks in {rows} rows", m.len());
            assert!(m.len() >= rows * (per_row - 8), "{} marks in {rows} rows", m.len());
            assert!(
                m.iter().all(|m| (from - 3. ..to + 3.).contains(&(m.3 as f32))),
                "scrolled to {scroll}: columns {from} to {to}, marks {:?}",
                m.iter().take(4).collect::<Vec<_>>()
            );
        }
    }

    /// The WCAG contrast ratio of two colours.
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let luminance = |color: Hsla| {
            let rgb = color.to_rgb();
            let linear = |v: f32| match v <= 0.03928 {
                true => v / 12.92,
                false => ((v + 0.055) / 1.055).powf(2.4),
            };
            0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
        };
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn marks_on_selected_text_are_the_guide_colour_mixed_halfway_to_the_foreground() {
        let (plain, selected) = mark_colors(c(theme::GUIDE), c(theme::EDITOR_FG));
        // plain text keeps the indent guides' colour
        assert_eq!(plain, c(theme::GUIDE));
        // Mariana: the guide, (73, 81, 90), halfway to the foreground, (216, 222, 233)
        let mixed = selected.to_rgb();
        for (got, want) in [mixed.r, mixed.g, mixed.b].into_iter().zip([144.5, 151.5, 161.5]) {
            assert!((got * 255. - want).abs() < 0.5, "{mixed:?}");
        }
        assert_eq!(mixed.a, 1.);
        // against the selection the guide colour is lost (1.11:1) and the mix reads
        let selection = c(theme::SELECTION);
        let (lost, read) = (contrast(plain, selection), contrast(selected, selection));
        assert!(lost < 1.2, "the guide colour on the selection: {lost}");
        assert!(read > 2. && read > 2. * lost, "{read} against {lost}");
    }

    #[test]
    fn marks_on_selected_text_are_mixed_toward_the_foreground_of_a_light_scheme_too() {
        // a light scheme: a pale guide, a dark foreground and a pale blue selection
        let (guide, foreground, selection) = (c(0xd3d3d3), c(0x202020), c(0xc8d4e8));
        let (plain, selected) = mark_colors(guide, foreground);
        assert_eq!(plain, guide);
        // darker than the guide, so it is the guide that is lost on the selection
        assert!(selected.to_rgb().r < guide.to_rgb().r);
        assert!(contrast(selected, selection) > 2. * contrast(guide, selection));
        // a scheme's own `guide` feeds both colours
        let (plain, selected) = mark_colors(c(0x884400), c(0xd8dee9));
        assert_eq!(plain, c(0x884400));
        assert_ne!(selected, c(0x884400));
    }

    /// The last frame's marks as (char, mark, colour).
    fn colored(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<(usize, Mark, Hsla)> {
        e.read_with(cx, |e, _| {
            let drawn = &e.layout.as_ref().unwrap().drawn;
            (drawn.marks.iter()).map(|&(at, mark, _, color)| (at, mark, color)).collect()
        })
    }

    #[gpui::test]
    fn a_mark_in_a_selection_is_mixed_toward_the_foreground_and_one_outside_keeps_the_guide_colour(
        cx: &mut TestAppContext,
    ) {
        let (guide, mixed) = mark_colors(c(theme::GUIDE), c(theme::EDITOR_FG));
        assert_ne!(guide, mixed);
        // 0 a, 1 b, 2 ␠, 3 c, 4 d, 5 ⇥, 6 e, 7 f, 8 ␠, 9 ⏎, 10 g, 11 h, 12 ␠, 13 i, 14 j, 15 ⏎
        let text = "ab cd\tef \ngh ij\n";
        let (e, cx) = white_editor(text, WhiteSpace::ALL, None, cx);
        // with nothing selected all of it is the guide colour
        let all = |color| {
            vec![
                (2, Space, color),
                (5, Tab, color),
                (8, Space, color),
                (9, Eol, color),
                (12, Space, color),
                (15, Eol, color),
            ]
        };
        assert_eq!(colored(&e, cx), all(guide));
        // a selection of the first line's spaces and tab, up to its break: the break is out
        redraw(&e, cx, |e, cx| e.sel(cx).set_selection(2, 9));
        assert_eq!(
            colored(&e, cx),
            [
                (2, Space, mixed),
                (5, Tab, mixed),
                (8, Space, mixed),
                (9, Eol, guide),
                (12, Space, guide),
                (15, Eol, guide),
            ]
        );
        // one that takes the break in colours the line end too, and backwards is the same
        for (a, b) in [(2, 11), (11, 2)] {
            redraw(&e, cx, move |e, cx| e.sel(cx).set_selection(a, b));
            assert_eq!(
                colored(&e, cx),
                [
                    (2, Space, mixed),
                    (5, Tab, mixed),
                    (8, Space, mixed),
                    (9, Eol, mixed),
                    (12, Space, guide),
                    (15, Eol, guide),
                ]
            );
        }
        // a selection of nothing but a break
        redraw(&e, cx, |e, cx| e.sel(cx).set_selection(9, 10));
        let m = colored(&e, cx);
        assert_eq!((m[3], m[2].2, m[4].2), ((9, Eol, mixed), guide, guide));
        // a selection starting at a line end, over the next line and its break
        redraw(&e, cx, |e, cx| e.sel(cx).set_selection(10, 16));
        assert_eq!(
            colored(&e, cx),
            [
                (2, Space, guide),
                (5, Tab, guide),
                (8, Space, guide),
                (9, Eol, guide),
                (12, Space, mixed),
                (15, Eol, mixed),
            ]
        );
        // carets colour nothing, and two selections colour each their own
        redraw(&e, cx, |e, cx| {
            e.sel(cx).set_selections(vec![Selection::new(1, 3), Selection::caret(8), Selection::new(12, 13)])
        });
        assert_eq!(
            colored(&e, cx),
            [
                (2, Space, mixed),
                (5, Tab, guide),
                (8, Space, guide),
                (9, Eol, guide),
                (12, Space, mixed),
                (15, Eol, guide),
            ]
        );
    }

    #[gpui::test]
    fn marks_in_the_find_panels_current_match_keep_the_guide_colour(cx: &mut TestAppContext) {
        let (guide, mixed) = mark_colors(c(theme::GUIDE), c(theme::EDITOR_FG));
        // 2 ␠, 5 ⇥, 8 ␠, 9 ⏎, 12 ␠, 15 ⏎; the current match " cd\t" is filled, not selected
        let text = "ab cd\tef \ngh ij\n";
        let (e, cx) = white_editor(text, WhiteSpace::ALL, None, cx);
        redraw(&e, cx, |e, cx| {
            e.find_ranges = vec![2..6];
            e.sel(cx).set_selections(vec![Selection::new(2, 6), Selection::new(12, 13)]);
        });
        assert_eq!(
            colored(&e, cx),
            [
                (2, Space, guide),
                (5, Tab, guide),
                (8, Space, guide),
                (9, Eol, guide),
                (12, Space, mixed),
                (15, Eol, guide),
            ]
        );
    }

    #[gpui::test]
    fn the_line_of_a_tab_and_the_marks_of_the_selection_mode_follow_the_same_colours(cx: &mut TestAppContext) {
        let (guide, mixed) = mark_colors(c(theme::GUIDE), c(theme::EDITOR_FG));
        assert_ne!(guide, mixed);
        // the tabs at columns 5 (3 wide) and 0 (4 wide)
        let text = "ab cd\tef \n\tgh\n";
        let (e, cx) = white_editor(text, WhiteSpace::ALL, None, cx);
        // the colours the lines are painted in
        let lines = |e: &Entity<Editor>, cx: &mut VisualTestContext| -> Vec<gpui::Background> {
            e.read_with(cx, |e, _| {
                let drawn = &e.layout.as_ref().unwrap().drawn;
                drawn.tab_lines.iter().map(|quad| quad.background).collect()
            })
        };
        let (guide_line, mixed_line) = (gpui::Background::from(guide), gpui::Background::from(mixed));
        assert_eq!(lines(&e, cx), [guide_line, guide_line]);
        // the first tab selected: its line is the colour of its arrow, the other tab's is not
        redraw(&e, cx, |e, cx| e.sel(cx).set_selection(4, 7));
        assert_eq!(lines(&e, cx), [mixed_line, guide_line]);
        redraw(&e, cx, |e, cx| e.sel(cx).set_selection(0, 14));
        assert_eq!(lines(&e, cx), [mixed_line, mixed_line]);
        // in "selection" every mark drawn is on selected text
        redraw(&e, cx, |e, cx| {
            e.view.white_space = WhiteSpace::SELECTION;
            e.sel(cx).set_selection(4, 12);
        });
        let m = colored(&e, cx);
        assert_eq!(m.len(), 3, "the tabs and the space, no line end: {m:?}");
        assert!(m.iter().all(|m| m.2 == mixed), "{m:?}");
        assert_eq!(lines(&e, cx), [mixed_line, mixed_line]);
    }

    #[gpui::test]
    fn a_folded_headers_line_end_is_selected_with_the_break_after_its_fold(cx: &mut TestAppContext) {
        let (guide, mixed) = mark_colors(c(theme::GUIDE), c(theme::EDITOR_FG));
        assert_ne!(guide, mixed);
        // lines 1 and 2 fold under the header: the break it ends with is the one at 22
        let text = "fn a() { \n    x \n    y\n}\nlast \n";
        let (e, cx) = white_editor(text, WhiteSpace::ALL, None, cx);
        redraw(&e, cx, |e, cx| e.fold_lines_now(0, 2, cx));
        let eol = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            let m = colored(e, cx);
            m.into_iter().find(|m| m.1 == Eol).map(|m| m.2)
        };
        assert_eq!(eol(&e, cx), Some(guide));
        // through the fold's text but not its break
        redraw(&e, cx, |e, cx| e.sel(cx).set_selection(8, 22));
        assert_eq!(eol(&e, cx), Some(guide));
        // taking the break in
        redraw(&e, cx, |e, cx| e.sel(cx).set_selection(8, 23));
        assert_eq!(eol(&e, cx), Some(mixed));
        // the header's own space is the selection's either way
        let m = colored(&e, cx);
        assert_eq!(m[2], (8, Space, mixed));
    }

    #[gpui::test]
    fn a_long_line_with_a_fold_is_shaped_around_the_view_and_follows_its_caret(cx: &mut TestAppContext) {
        // 200,000 chars on a line, then a short one
        let text = format!("{}\nend\n", "a ".repeat(100_000));
        let (e, cx) = white_editor(&text, WhiteSpace::NONE, None, cx);
        let shaped = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.read_with(cx, |e, _| {
                let l = e.layout.as_ref().unwrap();
                l.rows[0].as_ref().unwrap().1.len()
            })
        };
        // a short fold near its start: the row shows nearly all of it, and is shaped in part
        redraw(&e, cx, |e, cx| e.fold_chars_now([10..20], cx));
        assert!(shaped(&e, cx) < 5_000, "{} bytes shaped", shaped(&e, cx));
        // a long fold left of the caret: the view follows the caret by what is shown
        redraw(&e, cx, |e, cx| {
            e.fold_chars_now([10..150_010], cx);
            e.sel(cx).set_cursor(200_000);
            e.follow_caret = true;
        });
        assert!(shaped(&e, cx) < 5_000, "{} bytes shaped", shaped(&e, cx));
        e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            let caret = l.drawn.carets[0];
            let (a, b) = (l.text_area.left(), l.text_area.right());
            assert!(caret.left() >= a && caret.left() <= b, "{caret:?} in {a:?}..{b:?}");
        });
    }

    fn warning_offsets(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<usize> {
        e.read_with(cx, |e, _| e.layout.as_ref().unwrap().drawn.unicode_warnings.iter().map(|w| w.0).collect())
    }

    #[gpui::test]
    fn unicode_warning_overlays_preserve_shaping_wrap_selection_carets_and_hit_testing(cx: &mut TestAppContext) {
        let text = "a\u{a0}\te\u{301}👩\u{200d}👧\u{fe0f}\nאב\u{200b}cd\u{202e}ef\u{e000}";
        let (e, cx) = white_editor(text, WhiteSpace::ALL, Some(12), cx);
        redraw(&e, cx, |e, cx| {
            e.view.unicode_warnings = false;
            e.sel(cx).set_selections(vec![Selection::new(0, 5), Selection::caret(14)]);
        });
        let geometry = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.read_with(cx, |e, cx| {
                let l = e.layout.as_ref().unwrap();
                let rows: Vec<_> = l
                    .rows
                    .iter()
                    .map(|row| {
                        row.as_ref().map(|(col, line)| {
                            let xs: Vec<_> = line.text.char_indices().map(|(byte, _)| line.x_for_index(byte)).collect();
                            (*col, line.text.to_string(), line.width, xs)
                        })
                    })
                    .collect();
                let bounds: Vec<_> = (0..e.doc(cx).len()).map(|at| e.char_bounds(at, cx)).collect();
                let hits: Vec<_> = bounds.iter().map(|b| b.and_then(|b| e.hit(b.origin, false, cx))).collect();
                (rows, bounds, hits, l.drawn.carets.clone(), l.drawn.selected.clone(), e.state.selections.clone())
            })
        };
        let before = geometry(&e, cx);
        assert!(!crate::views::editor::ViewSettings::default().unicode_warnings);
        assert!(warning_offsets(&e, cx).is_empty());
        redraw(&e, cx, |e, _| e.view.unicode_warnings = true);
        assert_eq!(geometry(&e, cx), before);
        assert_eq!(warning_offsets(&e, cx), [1, 12, 15]);
        e.read_with(cx, |e, cx| {
            assert_eq!(e.doc(cx).text(), text);
            let l = e.layout.as_ref().unwrap();
            for &(at, glyph, bounds) in &l.drawn.unicode_warnings {
                assert_eq!(glyph, '!');
                assert_eq!(bounds.origin, e.char_bounds(at, cx).unwrap().origin);
                assert!(bounds.size.width + px(0.01) >= l.space_w);
            }
        });
        redraw(&e, cx, |e, _| {
            e.view.unicode_warning_characters = ['\u{301}', '\u{200d}']
                .into_iter()
                .map(|ch| {
                    (
                        ch,
                        crate::config::settings::unicode::UnicodeWarning {
                            glyph: '?',
                            description: "Explicit override".into(),
                        },
                    )
                })
                .collect();
        });
        assert_eq!(warning_offsets(&e, cx), [4, 6]);
        assert_eq!(geometry(&e, cx), before, "even explicit joiner/combining warnings only paint overlays");
        e.read_with(cx, |e, _| assert!(e.layout.as_ref().unwrap().drawn.unicode_warnings.iter().all(|w| w.1 == '?')));
        redraw(&e, cx, |e, _| {
            e.view.unicode_warning_characters.clear();
            e.view.unicode_warning_ranges = serde_json::from_str(
                r#"{
                "U+00A0":{"glyph":"!","description":"NBSP"},
                "U+0300..U+036F":{"glyph":"?","description":"Accents"},
                "U+2000..U+202F":{"glyph":"!","description":"Invisible controls"},
                "U+E000":{"glyph":"?","description":"Private marker"}}
            "#,
            )
            .unwrap();
        });
        assert_eq!(warning_offsets(&e, cx), [1, 4, 6, 12, 15, 18]);
        assert_eq!(geometry(&e, cx), before, "range-only warnings retain every shaped position");
        let whites = e.read_with(cx, |e, _| e.layout.as_ref().unwrap().drawn.marks.clone());
        redraw(&e, cx, |e, _| e.view.unicode_warnings = false);
        assert!(warning_offsets(&e, cx).is_empty());
        assert_eq!(geometry(&e, cx), before);
        e.read_with(cx, |e, _| assert_eq!(e.layout.as_ref().unwrap().drawn.marks, whites));
        redraw(&e, cx, |e, _| {
            e.view.white_space = WhiteSpace::NONE;
            e.view.unicode_warnings = true;
        });
        assert_eq!(warning_offsets(&e, cx), [1, 4, 6, 12, 15, 18]);
        e.read_with(cx, |e, _| assert!(e.layout.as_ref().unwrap().drawn.marks.is_empty()));
    }

    #[gpui::test]
    fn unicode_warning_scanning_is_clipped_and_omits_hidden_folds(cx: &mut TestAppContext) {
        let text = format!("a\u{a0}b\u{200b}c\u{202e}{}\u{feff}", "x".repeat(200_000));
        let (e, cx) = white_editor(&text, WhiteSpace::NONE, None, cx);
        redraw(&e, cx, |e, cx| {
            e.view.unicode_warnings = true;
            e.view.unicode_warning_characters.clear();
            e.view.unicode_warning_ranges = serde_json::from_str(
                r#"{
                "U+00A0":{"glyph":"!","description":"NBSP"},
                "U+200B":{"glyph":"!","description":"Zero width space"},
                "U+202E":{"glyph":"!","description":"Direction control"},
                "U+FEFF":{"glyph":"!","description":"BOM"}}
            "#,
            )
            .unwrap();
            e.sel(cx).set_cursor(0);
            e.fold_chars_now([2..5], cx);
            e.update_doc(cx, |d| d.read_only = true);
        });
        assert_eq!(warning_offsets(&e, cx), [1, 5]);
        e.read_with(cx, |e, cx| {
            assert!(e.layout.as_ref().unwrap().drawn.unicode_visits < 5_000);
            assert_eq!(e.doc(cx).text(), text);
        });
        redraw(&e, cx, |e, cx| {
            let end = e.doc(cx).len();
            e.sel(cx).set_cursor(end);
            e.follow_caret = true;
        });
        assert_eq!(warning_offsets(&e, cx), [200_006]);
        e.read_with(cx, |e, _| assert!(e.layout.as_ref().unwrap().drawn.unicode_visits < 5_000));
        redraw(&e, cx, |e, cx| {
            e.fold_chars_now([2..200_006], cx);
            e.sel(cx).set_cursor(0);
            e.follow_caret = true;
        });
        assert_eq!(warning_offsets(&e, cx), [1, 200_006]);
        e.read_with(cx, |e, _| assert!(e.layout.as_ref().unwrap().drawn.unicode_visits < 8));
        redraw(&e, cx, |e, _| {
            e.view.unicode_warning_characters.clear();
            e.view.unicode_warning_ranges.clear();
        });
        assert!(warning_offsets(&e, cx).is_empty());
    }

    #[gpui::test]
    fn unicode_warning_ime_suppression_commit_cancel_and_undo(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let composed = "\u{a0}e\u{301}👩\u{200d}👧\u{fe0f}";
        let (e, cx) = white_editor("x", WhiteSpace::NONE, None, cx);
        redraw(&e, cx, |e, cx| {
            e.view.unicode_warnings = true;
            e.sel(cx).set_cursor(1);
        });
        e.update_in(cx, |e, window, cx| {
            e.replace_and_mark_text_in_range(None, composed, Some(1..3), window, cx);
            assert_eq!(e.marked_text_range(window, cx), Some(1..10));
        });
        cx.run_until_parked();
        assert!(warning_offsets(&e, cx).is_empty(), "composition is not annotated");
        let geometry = e.read_with(cx, |e, cx| e.char_bounds(1, cx));
        e.update_in(cx, |e, window, cx| e.replace_text_in_range(None, composed, window, cx));
        cx.run_until_parked();
        assert_eq!(warning_offsets(&e, cx), [1]);
        e.read_with(cx, |e, cx| {
            assert_eq!(e.doc(cx).text(), format!("x{composed}"));
            assert_eq!(e.char_bounds(1, cx), geometry);
        });
        redraw(&e, cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert!(warning_offsets(&e, cx).is_empty());
        e.read_with(cx, |e, cx| assert_eq!(e.doc(cx).text(), "x"));
        redraw(&e, cx, |e, cx| e.edit(cx, |s| s.redo()));
        assert_eq!(warning_offsets(&e, cx), [1]);
        e.update_in(cx, |e, window, cx| {
            e.replace_and_mark_text_in_range(None, "\u{200b}", None, window, cx);
            e.replace_and_mark_text_in_range(None, "", None, window, cx);
            assert!(e.marked_text_range(window, cx).is_none());
        });
        cx.run_until_parked();
        assert_eq!(warning_offsets(&e, cx), [1], "cancelled text leaves no stale warning");
        e.update_in(cx, |e, window, cx| e.replace_and_mark_text_in_range(None, "\u{200b}", None, window, cx));
        cx.run_until_parked();
        assert_eq!(warning_offsets(&e, cx), [1]);
        e.update_in(cx, |e, window, cx| e.unmark_text(window, cx));
        cx.run_until_parked();
        assert_eq!(warning_offsets(&e, cx).len(), 2, "ending composition redraws its newly committed warning");
    }

    #[gpui::test]
    fn unicode_warning_ranges_preserve_native_ime_coordinates_and_commit(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let composed = "\u{200b}e\u{301}\u{e000}😀";
        let (e, cx) = white_editor("x", WhiteSpace::NONE, Some(8), cx);
        e.update_in(cx, |e, window, cx| {
            e.sel(cx).set_cursor(1);
            e.view.unicode_warning_characters.clear();
            e.view.unicode_warning_ranges = serde_json::from_str(
                r#"{
                "U+0300..U+036F":{"glyph":"?","description":"Accent"},
                "U+200B":{"glyph":"!","description":"Zero width"},
                "U+E000":{"glyph":"!","description":"Private"}}
            "#,
            )
            .unwrap();
            e.replace_and_mark_text_in_range(None, composed, Some(1..3), window, cx);
        });
        cx.run_until_parked();
        let native = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.update_in(cx, |e, window, cx| {
                let marked = e.marked_text_range(window, cx);
                let selected = e.selected_text_range(false, window, cx).map(|s| s.range);
                let bounds = e.bounds_for_range(1..4, Bounds::default(), window, cx);
                let hit = bounds.and_then(|b| e.character_index_for_point(b.origin, window, cx));
                let text = e.text_for_range(0..7, &mut None, window, cx);
                (marked, selected, bounds, hit, text)
            })
        };
        let before = native(&e, cx);
        assert_eq!(before.0, Some(1..7));
        assert!(before.2.is_some());
        assert!(warning_offsets(&e, cx).is_empty());
        redraw(&e, cx, |e, _| e.view.unicode_warnings = true);
        assert_eq!(native(&e, cx), before);
        assert!(warning_offsets(&e, cx).is_empty(), "range marks suppress the complete active composition");
        e.update_in(cx, |e, window, cx| e.replace_text_in_range(None, composed, window, cx));
        cx.run_until_parked();
        assert_eq!(warning_offsets(&e, cx), [1, 3, 4]);
        let committed = native(&e, cx);
        redraw(&e, cx, |e, _| e.view.unicode_warnings = false);
        assert_eq!(native(&e, cx), committed);
        redraw(&e, cx, |e, cx| e.edit(cx, |s| s.undo()));
        e.read_with(cx, |e, cx| assert_eq!(e.doc(cx).text(), "x"));
        redraw(&e, cx, |e, cx| e.edit(cx, |s| s.redo()));
        e.read_with(cx, |e, cx| assert_eq!(e.doc(cx).text(), format!("x{composed}")));
    }

    #[gpui::test]
    fn unicode_warning_many_ranges_scan_only_clipped_source(cx: &mut TestAppContext) {
        let text = format!("\u{e000}{}", "x".repeat(200_000));
        let (e, cx) = white_editor(&text, WhiteSpace::NONE, None, cx);
        redraw(&e, cx, |e, _| {
            e.view.unicode_warnings = true;
            e.view.unicode_warning_characters.clear();
            let mut values = serde_json::Map::new();
            for start in 0xE000..0xF000 {
                values.insert(
                    format!("U+{start:04X}..U+10FFFF"),
                    serde_json::json!({"glyph":"!","description":"Overlapping interval"}),
                );
            }
            e.view.unicode_warning_ranges = serde_json::from_value(values.into()).unwrap();
        });
        assert_eq!(warning_offsets(&e, cx), [0]);
        e.read_with(cx, |e, cx| {
            assert_eq!(e.view.unicode_warning_ranges.len(), 4096);
            assert!(e.layout.as_ref().unwrap().drawn.unicode_visits < 5000);
            assert_eq!(e.doc(cx).text(), text);
        });
    }

    #[gpui::test]
    fn unicode_warning_many_expired_ranges_keep_visible_lookup_bounded(cx: &mut TestAppContext) {
        let text = "\u{f100}".repeat(200_000);
        let (e, cx) = white_editor(&text, WhiteSpace::NONE, None, cx);
        for live_lower_interval in [false, true] {
            redraw(&e, cx, move |e, _| {
                e.view.unicode_warnings = true;
                e.view.unicode_warning_characters.clear();
                let mut values = serde_json::Map::new();
                for start in 0xE001..0xF001 {
                    values.insert(
                        format!("U+{start:04X}"),
                        serde_json::json!({"glyph":"!","description":"Expired singleton"}),
                    );
                }
                if live_lower_interval {
                    values.insert(
                        "U+E000..U+10FFFF".into(),
                        serde_json::json!({"glyph":"?","description":"Lower live interval"}),
                    );
                }
                e.view.unicode_warning_ranges = serde_json::from_value(values.into()).unwrap();
            });
            let start = std::time::Instant::now();
            for _ in 0..10 {
                redraw(&e, cx, |_, _| {});
            }
            eprintln!(
                "4096 expired ranges, lower live={live_lower_interval}: ten clipped frames {:?}",
                start.elapsed()
            );
            assert_eq!(warning_offsets(&e, cx).is_empty(), !live_lower_interval);
            e.read_with(cx, |e, cx| {
                assert!(e.layout.as_ref().unwrap().drawn.unicode_visits < 5000);
                assert_eq!(e.doc(cx).text(), text);
            });
        }
    }

    #[gpui::test]
    fn unicode_warning_glyphs_are_absent_from_single_line_widgets(cx: &mut TestAppContext) {
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("a\u{a0}b"), true, cx));
        redraw(&e, cx, |e, _| e.view.unicode_warnings = true);
        assert!(warning_offsets(&e, cx).is_empty());
        e.read_with(cx, |e, _| assert_eq!(e.layout.as_ref().unwrap().drawn.unicode_visits, 0));
    }

    #[gpui::test]
    fn unicode_warning_colors_follow_custom_surfaces(cx: &mut TestAppContext) {
        // Themes are process-global. Isolate this real palette/renderer test from parallel tests.
        const ISOLATED: &str = "ORLO_UNICODE_COLOR_TEST";
        if std::env::var_os(ISOLATED).is_none() {
            let name = format!(
                "{}::unicode_warning_colors_follow_custom_surfaces",
                module_path!().split_once("::").unwrap().1
            );
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &name, "--nocapture"])
                .env(ISOLATED, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        use syntect::highlighting::{Color, ThemeSettings};
        let white = Color { r: 255, g: 255, b: 255, a: 255 };
        let black = Color { r: 0, g: 0, b: 0, a: 255 };
        let settings = |caret, alpha| ThemeSettings {
            background: Some(white),
            foreground: Some(black),
            caret: Some(caret),
            selection: Some(Color { a: alpha, ..black }),
            selection_foreground: Some(white),
            find_highlight: Some(Color { r: 0, g: 0, b: 80, a: 255 }),
            ..Default::default()
        };
        theme::apply_scheme(&settings(black, 255));
        let text = "\u{a0} \u{a0} \u{200b} \u{2028} \u{2029}";
        let (e, cx) = white_editor(text, WhiteSpace::NONE, None, cx);
        for (caret, alpha) in [(black, 255), (white, 255), (black, 192)] {
            theme::apply_scheme(&settings(caret, alpha));
            redraw(&e, cx, |e, cx| {
                e.view.unicode_warnings = true;
                e.sel(cx).set_selections(vec![Selection::new(2, 3), Selection::new(6, 7)]);
                e.find_ranges = vec![6..7];
            });
            assert_eq!(warning_offsets(&e, cx), [0, 2, 4, 6, 8], "Unicode separators are displayed warning scalars");
            e.read_with(cx, |e, cx| {
                assert_eq!(e.doc(cx).text(), text);
                let drawn = &e.layout.as_ref().unwrap().drawn;
                assert!(drawn.unicode_warnings.iter().all(|w| w.1 == '!'));
                for (&(at, _, _), &color) in drawn.unicode_warnings.iter().zip(&drawn.unicode_warning_colors) {
                    let surface = match at {
                        2 => e.background().blend(c(theme::SELECTION)),
                        6 => e.background().blend(c(theme::FIND_CURRENT_BG)),
                        _ => e.background(),
                    };
                    assert!(contrast(color, surface) >= 4.5, "warning at {at}: {color:?} against {surface:?}");
                }
                assert_ne!(
                    drawn.unicode_warning_colors[0], drawn.unicode_warning_colors[1],
                    "same glyph needs different cached colors"
                );
            });
        }
    }
}
