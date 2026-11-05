//! Sublime's `set_layout` model (columns, rows, cells) and the bookkeeping of which tabs sit
//! in which group. Generic over the tab type and independent of a presentation runtime.

use serde::{Deserialize, Serialize};

/// Column and row boundaries as fractions of the editor area, and each group's cell as
/// `[col0, row0, col1, row1]` indices into them, exactly as Sublime's `set_layout` takes them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub cols: Vec<f32>,
    pub rows: Vec<f32>,
    pub cells: Vec<[usize; 4]>,
}

impl Default for Layout {
    fn default() -> Self {
        Self::preset(Preset::Single)
    }
}

/// View > Layout. Columns 2..=4 and Rows 2..=3, as Sublime offers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Single,
    Columns(usize),
    Rows(usize),
    Grid4,
}

/// Sublime's keymap boundaries: thirds are 0.33 / 0.66, not exact.
fn splits(n: usize) -> Vec<f32> {
    match n {
        2 => vec![0., 0.5, 1.],
        3 => vec![0., 0.33, 0.66, 1.],
        4 => vec![0., 0.25, 0.5, 0.75, 1.],
        _ => vec![0., 1.],
    }
}

impl Layout {
    pub fn preset(p: Preset) -> Self {
        let (cols, rows, cells) = match p {
            Preset::Single => (splits(1), splits(1), vec![[0, 0, 1, 1]]),
            Preset::Columns(n) => (splits(n), splits(1), (0..n).map(|i| [i, 0, i + 1, 1]).collect()),
            Preset::Rows(n) => (splits(1), splits(n), (0..n).map(|i| [0, i, 1, i + 1]).collect()),
            Preset::Grid4 => (splits(2), splits(2), vec![[0, 0, 1, 1], [1, 0, 2, 1], [0, 1, 1, 2], [1, 1, 2, 2]]),
        };
        Self { cols, rows, cells }
    }

    pub fn groups(&self) -> usize {
        self.cells.len()
    }

    /// A layout a window can draw (a session file is hand-editable): boundaries from 0 to 1,
    /// ascending; cells that tile the area exactly, with no gap or overlap. A boundary need not
    /// be a full line: one tall group beside two stacked ones is a layout.
    pub fn is_valid(&self) -> bool {
        let edges = |v: &[f32]| {
            v.len() >= 2 && v.first() == Some(&0.) && v.last() == Some(&1.) && v.windows(2).all(|w| w[0] < w[1])
        };
        if !edges(&self.cols) || !edges(&self.rows) || self.cells.is_empty() {
            return false;
        }
        let (nc, nr) = (self.cols.len() - 1, self.rows.len() - 1);
        // a hand-edited file: no grid bigger than a screen could show
        if nc > 64 || nr > 64 || self.cells.len() > 64 {
            return false;
        }
        let mut covered = vec![false; nc * nr];
        for &[c0, r0, c1, r1] in &self.cells {
            if !(c0 < c1 && c1 <= nc && r0 < r1 && r1 <= nr) {
                return false;
            }
            for c in c0..c1 {
                for r in r0..r1 {
                    // two groups over one cell
                    if std::mem::replace(&mut covered[r * nc + c], true) {
                        return false;
                    }
                }
            }
        }
        covered.iter().all(|&c| c)
    }

    /// `[x0, y0, x1, y1]` of group `g` as fractions of the editor area.
    pub fn cell(&self, g: usize) -> [f32; 4] {
        let [c0, r0, c1, r1] = self.cells[g];
        [self.cols[c0], self.rows[r0], self.cols[c1], self.rows[r1]]
    }
}

/// Which boundary list a divider moves: a vertical line at `cols[i]` or a horizontal one at `rows[i]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Cols,
    Rows,
}

impl Axis {
    /// The indices of a cell's two edges on this axis: `c0, c1` or `r0, r1`.
    fn edges(self) -> (usize, usize) {
        match self {
            Axis::Cols => (0, 2),
            Axis::Rows => (1, 3),
        }
    }

    fn other(self) -> Axis {
        match self {
            Axis::Cols => Axis::Rows,
            Axis::Rows => Axis::Cols,
        }
    }
}

/// One draggable divider: grid line `index` of `axis`, drawn from `from` to `to` (fractions
/// along the other axis) where the groups on its two sides meet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Divider {
    pub axis: Axis,
    pub index: usize,
    pub from: f32,
    pub to: f32,
}

impl Layout {
    fn lines(&self, axis: Axis) -> &Vec<f32> {
        match axis {
            Axis::Cols => &self.cols,
            Axis::Rows => &self.rows,
        }
    }

    fn lines_mut(&mut self, axis: Axis) -> &mut Vec<f32> {
        match axis {
            Axis::Cols => &mut self.cols,
            Axis::Rows => &mut self.rows,
        }
    }

    /// The dividers: each interior grid line, cut into the stretches where groups meet across
    /// it (a line a group spans over is no divider there).
    pub fn dividers(&self) -> Vec<Divider> {
        let mut out = Vec::new();
        for axis in [Axis::Cols, Axis::Rows] {
            let ((_, end), (x, y)) = (axis.edges(), axis.other().edges());
            let along = self.lines(axis.other());
            for i in 1..self.lines(axis).len().saturating_sub(1) {
                // the spans of the groups ending at line i, merged where they touch
                let mut spans: Vec<(usize, usize)> =
                    (self.cells.iter()).filter_map(|c| (c[end] == i).then_some((c[x], c[y]))).collect();
                spans.sort();
                let mut merged: Vec<(usize, usize)> = Vec::new();
                for (a, b) in spans {
                    match merged.last_mut() {
                        Some(last) if last.1 == a => last.1 = b,
                        _ => merged.push((a, b)),
                    }
                }
                out.extend(merged.into_iter().map(|(a, b)| Divider { axis, index: i, from: along[a], to: along[b] }));
            }
        }
        out
    }

    /// Move interior boundary `i` to fraction `to`, kept `min` away from its neighbours, so no
    /// group gets narrower than `min`.
    pub fn drag(&mut self, axis: Axis, i: usize, to: f32, min: f32) {
        let v = self.lines_mut(axis);
        if i == 0 || i + 1 >= v.len() {
            return;
        }
        let (lo, hi) = (v[i - 1] + min, v[i + 1] - min);
        if lo <= hi {
            v[i] = to.clamp(lo, hi);
        }
    }

    /// Group `g` split in two along `axis` (`Cols`: side by side), the new group right after
    /// it as group `g + 1`. A grid line already inside the group is used, so the halves line up
    /// with their neighbours; else one is added at its middle. `None` when `g` does not exist.
    pub fn split(&self, g: usize, axis: Axis) -> Option<Layout> {
        let (a, b) = axis.edges();
        let (lo, hi) = (self.cells.get(g)?[a], self.cells[g][b]);
        let mut l = self.clone();
        let v = l.lines_mut(axis);
        let mid = (v[lo] + v[hi]) / 2.;
        // an existing line inside the group, the nearest to its middle
        let inside = (lo + 1..hi).min_by(|&p, &q| (v[p] - mid).abs().total_cmp(&(v[q] - mid).abs()));
        let k = match inside {
            Some(k) => k,
            None => {
                v.insert(lo + 1, mid);
                // every index at or past the new line moves along
                for cell in &mut l.cells {
                    for j in [a, b] {
                        if cell[j] > lo {
                            cell[j] += 1;
                        }
                    }
                }
                lo + 1
            }
        };
        let (mut first, mut second) = (l.cells[g], l.cells[g]);
        (first[b], second[a]) = (k, k);
        l.cells[g] = first;
        l.cells.insert(g + 1, second);
        Some(l)
    }

    /// Group `g` taken out, the groups on one side of it growing over its place: the ones
    /// whose edge against it covers it exactly (left first, then above, right, below). The new
    /// layout and the group that takes `g`'s views, numbered as in it; `None` when no side
    /// fits (or `g` is the only group).
    pub fn close(&self, g: usize) -> Option<(Layout, usize)> {
        if self.cells.len() < 2 {
            return None;
        }
        let cell = *self.cells.get(g)?;
        // (the axis of the side's line, whether that side is after `g`)
        let sides: [(Axis, bool); 4] =
            [(Axis::Cols, false), (Axis::Rows, false), (Axis::Cols, true), (Axis::Rows, true)];
        for (axis, after) in sides {
            // `a`, `b`: the edges on the axis; `x`, `y`: the span across it
            let ((a, b), (x, y)) = (axis.edges(), axis.other().edges());
            let touching: Vec<usize> = (0..self.cells.len())
                .filter(|&k| k != g)
                .filter(|&k| {
                    let n = self.cells[k];
                    let edge = if after { n[a] == cell[b] } else { n[b] == cell[a] };
                    edge && n[x] < cell[y] && n[y] > cell[x]
                })
                .collect();
            // each neighbour within g's span, together covering it
            let fits = !touching.is_empty()
                && touching.iter().all(|&k| {
                    let n = self.cells[k];
                    cell[x] <= n[x] && n[y] <= cell[y]
                })
                && (touching.iter()).map(|&k| self.cells[k][y] - self.cells[k][x]).sum::<usize>() == cell[y] - cell[x];
            if !fits {
                continue;
            }
            let mut l = self.clone();
            // the neighbours' facing edges move to g's far one
            let j = if after { a } else { b };
            for &k in &touching {
                l.cells[k][j] = cell[j];
            }
            l.cells.remove(g);
            l.compact();
            let first = touching.iter().copied().min().unwrap_or(0);
            let target = if first > g { first - 1 } else { first };
            return Some((l, target));
        }
        None
    }

    /// Grid lines no group's edge is on are dropped, and the indices after them moved down.
    fn compact(&mut self) {
        for axis in [Axis::Cols, Axis::Rows] {
            let (a, b) = axis.edges();
            let mut i = 1;
            while i + 1 < self.lines(axis).len() {
                if self.cells.iter().any(|c| c[a] == i || c[b] == i) {
                    i += 1;
                    continue;
                }
                self.lines_mut(axis).remove(i);
                for c in &mut self.cells {
                    for j in [a, b] {
                        if c[j] > i {
                            c[j] -= 1;
                        }
                    }
                }
            }
        }
    }

    /// Whether every group is at least `min_w` by `min_h` in an area `w` by `h`.
    pub fn fits(&self, w: f32, h: f32, min_w: f32, min_h: f32) -> bool {
        (0..self.cells.len()).all(|g| {
            let [x0, y0, x1, y1] = self.cell(g);
            (x1 - x0) * w >= min_w && (y1 - y0) * h >= min_h
        })
    }
}

/// A layout the user saved under a name, in `Layouts.json` beside the session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedLayout {
    pub name: String,
    #[serde(flatten)]
    pub layout: Layout,
}

/// `~/Library/Application Support/Orlo/Layouts.json`
pub fn saved_path() -> std::path::PathBuf {
    orlo_utils::paths::support_dir().join("Layouts.json")
}

/// The saved layouts at `path` (none when it does not exist), and why any were left out: a
/// hand-edited file is checked entry by entry.
pub fn load_saved(path: &std::path::Path) -> Result<(Vec<SavedLayout>, Vec<String>), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let v: serde_json::Value = serde_json::from_str(&orlo_config::settings::strip_json(&text))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let Some(list) = v.get("layouts").and_then(|l| l.as_array()) else {
        return Err(format!("{}: no \"layouts\" list", path.display()));
    };
    let (mut ok, mut bad) = (Vec::new(), Vec::new());
    for (i, entry) in list.iter().enumerate() {
        match serde_json::from_value::<SavedLayout>(entry.clone()) {
            Ok(l) if l.name.trim().is_empty() => bad.push(format!("layout {}: no name", i + 1)),
            Ok(l) if !l.layout.is_valid() => bad.push(format!("\"{}\": its cells do not tile the window", l.name)),
            Ok(l) if ok.iter().any(|o: &SavedLayout| o.name == l.name) => {
                bad.push(format!("\"{}\": the name is used twice", l.name))
            }
            Ok(l) => ok.push(l),
            Err(e) => bad.push(format!("layout {}: {e}", i + 1)),
        }
    }
    Ok((ok, bad))
}

/// Written to a temporary file renamed into place.
pub fn save_saved(path: &std::path::Path, list: &[SavedLayout]) -> std::io::Result<()> {
    let v = serde_json::json!({ "version": 1, "layouts": list });
    let text = serde_json::to_string_pretty(&v).map_err(std::io::Error::other)?;
    orlo_project::io::write_atomic(path, &text)
}

pub use crate::panes::Groups;
pub use crate::tabs::Group;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hand_edited_layout_with_a_cell_over_and_over_is_refused() {
        let mut l = Layout::preset(Preset::Columns(2));
        l.cells = vec![[0, 0, 1, 1]; 300];
        l.cells.push([1, 0, 2, 1]);
        assert!(!l.is_valid(), "one cell 300 times");
        let mut huge = Layout::default();
        huge.cols = (0..=100_000).map(|i| i as f32 / 100_000.).collect();
        huge.rows = huge.cols.clone();
        assert!(!huge.is_valid(), "a grid no screen shows");
    }

    #[test]
    fn only_layouts_whose_cells_fit_their_boundaries_are_valid() {
        for p in [Preset::Single, Preset::Columns(3), Preset::Rows(2), Preset::Grid4] {
            assert!(Layout::preset(p).is_valid());
        }
        let bad = |cols: Vec<f32>, rows: Vec<f32>, cells: Vec<[usize; 4]>| !Layout { cols, rows, cells }.is_valid();
        assert!(bad(vec![0., 1.], vec![0., 1.], vec![[0, 0, 2, 1]]), "cell past the columns");
        assert!(bad(vec![0., 1.], vec![0., 1.], vec![]), "no cells");
        assert!(bad(vec![0., 0.6, 0.4, 1.], vec![0., 1.], vec![[0, 0, 1, 1]]), "unsorted");
        assert!(bad(vec![0., 2.], vec![0., 1.], vec![[0, 0, 1, 1]]), "past the edge");
        assert!(bad(vec![0., 1.], vec![0., 1.], vec![[1, 0, 1, 1]]), "empty cell");
        let half = || vec![0., 0.5, 1.];
        assert!(bad(vec![0.1, 1.], vec![0., 1.], vec![[0, 0, 1, 1]]), "not from the edge");
        assert!(bad(half(), vec![0., 1.], vec![[0, 0, 1, 1]]), "a gap");
        assert!(bad(half(), vec![0., 1.], vec![[0, 0, 2, 1], [1, 0, 2, 1]]), "an overlap");
        assert!(
            Layout { cols: half(), rows: half(), cells: vec![[0, 0, 1, 2], [1, 0, 2, 1], [1, 1, 2, 2]] }.is_valid(),
            "one tall group beside two stacked ones"
        );
    }

    fn three_tabs() -> Groups<&'static str> {
        let mut g = Groups::new();
        for t in ["a", "b", "c"] {
            g.add(t);
        }
        g
    }

    fn grid(tabs: [&[&'static str]; 4], active: usize) -> Groups<&'static str> {
        let mut g = Groups::new();
        g.set_layout(Layout::preset(Preset::Grid4));
        for (i, ts) in tabs.iter().enumerate() {
            g.focus(i);
            for t in ts.iter() {
                g.add(*t);
            }
        }
        g.focus(active);
        g
    }

    #[test]
    fn presets_match_sublime_keymap() {
        assert_eq!(Layout::preset(Preset::Columns(3)).cols, vec![0., 0.33, 0.66, 1.]);
        assert_eq!(Layout::preset(Preset::Grid4).groups(), 4);
        assert_eq!(Layout::preset(Preset::Rows(2)).cell(1), [0., 0.5, 1., 1.]);
        assert_eq!(Layout::preset(Preset::Single).cell(0), [0., 0., 1., 1.]);
        assert_eq!(Layout::preset(Preset::Grid4).cell(3), [0.5, 0.5, 1., 1.]);
        assert_eq!(Layout::preset(Preset::Columns(4)).groups(), 4);
        assert_eq!(Layout::preset(Preset::Rows(3)).groups(), 3);
    }

    #[test]
    fn more_groups_focus_the_last_one() {
        let mut g = three_tabs();
        g.set_layout(Layout::preset(Preset::Columns(2)));
        assert_eq!(g.groups[0].tabs, vec!["a", "b", "c"]);
        assert!(g.groups[1].tabs.is_empty());
        assert_eq!(g.active, 1);
        assert_eq!(g.active_tab(), None);
        assert_eq!(g.groups[0].active, 2, "group 1 keeps its active tab");
    }

    #[test]
    fn fewer_groups_append_to_the_last_remaining_group() {
        let mut g = grid([&["d"], &["a"], &["b"], &["c"]], 2);
        g.set_layout(Layout::preset(Preset::Columns(2)));
        assert_eq!(g.groups.len(), 2);
        assert_eq!(g.groups[0].tabs, vec!["d"]);
        assert_eq!(g.groups[1].tabs, vec!["a", "b", "c"]);
        assert_eq!(g.active, 1);
        assert_eq!(g.active_tab(), Some(&"b"));
    }

    #[test]
    fn single_collects_everything_in_order_and_keeps_the_focused_view() {
        let mut g = Groups::new();
        g.set_layout(Layout::preset(Preset::Columns(2)));
        g.focus(0);
        g.add("b");
        g.focus(1);
        g.add("a");
        g.add("c");
        g.select(1, 0);
        g.set_layout(Layout::preset(Preset::Single));
        assert_eq!(g.groups.len(), 1);
        assert_eq!(g.groups[0].tabs, vec!["b", "a", "c"]);
        assert_eq!(g.active_tab(), Some(&"a"));
        // same number of groups keeps the focused group index
        let mut h = grid([&["d"], &["a"], &[], &[]], 1);
        h.set_layout(Layout::preset(Preset::Columns(4)));
        assert_eq!(h.active, 1);
    }

    #[test]
    fn move_appends_and_focus_follows() {
        let mut g = three_tabs();
        g.set_layout(Layout::preset(Preset::Columns(2)));
        g.focus(1);
        g.add("x");
        g.select(0, 1);
        g.move_active_to(1);
        assert_eq!(g.groups[0].tabs, vec!["a", "c"]);
        assert_eq!(g.groups[1].tabs, vec!["x", "b"]);
        assert_eq!(g.active, 1);
        assert_eq!(g.active_tab(), Some(&"b"));
    }

    #[test]
    fn move_or_focus_to_a_missing_group_is_a_noop() {
        let mut g = three_tabs();
        g.set_layout(Layout::preset(Preset::Columns(2)));
        g.select(0, 0);
        g.focus(2);
        assert_eq!(g.active, 0);
        g.move_active_to(2);
        g.move_active_to(0);
        assert_eq!(g.groups[0].tabs, vec!["a", "b", "c"]);
        assert_eq!(g.active_tab(), Some(&"a"));
        g.focus(1);
        g.move_active_to(0); // empty group: nothing to move
        assert_eq!(g.groups[0].tabs.len(), 3);
    }

    #[test]
    fn closing_the_last_tab_leaves_an_empty_focused_group() {
        let mut g = three_tabs();
        g.set_layout(Layout::preset(Preset::Columns(2)));
        g.focus(1);
        g.add("x");
        assert_eq!(g.remove(&"x"), Some((1, 0)));
        assert_eq!(g.groups.len(), 2);
        assert_eq!(g.active, 1);
        assert_eq!(g.active_tab(), None);
        assert_eq!(g.remove(&"nope"), None);
    }

    #[test]
    fn neighbour_wraps() {
        let mut g: Groups<&str> = Groups::new();
        g.set_layout(Layout::preset(Preset::Columns(3)));
        assert_eq!(g.active, 2);
        assert_eq!(g.neighbour(true), 0);
        g.focus(0);
        assert_eq!(g.neighbour(false), 2);
        assert_eq!(g.neighbour(true), 1);
    }

    #[test]
    fn remove_keeps_stage1_next_active_rule() {
        let mut g = three_tabs();
        g.select(0, 1);
        g.remove(&"a");
        assert_eq!(g.active_tab(), Some(&"b"));
        g.select(0, 1);
        g.remove(&"c");
        assert_eq!(g.active_tab(), Some(&"b"));
    }

    fn d(axis: Axis, index: usize, from: f32, to: f32) -> Divider {
        Divider { axis, index, from, to }
    }

    #[test]
    fn dividers_of_presets() {
        assert_eq!(Layout::preset(Preset::Grid4).dividers(), vec![d(Axis::Cols, 1, 0., 1.), d(Axis::Rows, 1, 0., 1.)]);
        assert_eq!(
            Layout::preset(Preset::Columns(3)).dividers(),
            vec![d(Axis::Cols, 1, 0., 1.), d(Axis::Cols, 2, 0., 1.)]
        );
        assert!(Layout::preset(Preset::Single).dividers().is_empty());
    }

    #[test]
    fn drag_moves_the_boundary_for_every_cell_on_it() {
        let mut l = Layout::preset(Preset::Grid4);
        l.drag(Axis::Cols, 1, 0.7, 0.05);
        assert_eq!(l.cell(0)[2], 0.7);
        assert_eq!(l.cell(2)[2], 0.7);
        assert_eq!(l.cell(1)[0], 0.7);
        l.drag(Axis::Rows, 1, 0.25, 0.05);
        assert_eq!(l.cell(3)[1], 0.25);
    }

    #[test]
    fn drag_is_clamped_by_neighbours() {
        let mut l = Layout::preset(Preset::Columns(3));
        l.drag(Axis::Cols, 1, 0.9, 0.05);
        assert!((l.cols[1] - 0.61).abs() < 1e-6);
        l.drag(Axis::Cols, 1, -1., 0.05);
        assert!((l.cols[1] - 0.05).abs() < 1e-6);
        // the outer edges never move
        l.drag(Axis::Cols, 0, 0.5, 0.05);
        assert_eq!(l.cols[0], 0.);
    }

    #[test]
    fn insert_next_goes_after_the_active_tab() {
        let mut g = three_tabs();
        g.select(0, 0);
        g.insert_next("a2");
        assert_eq!(g.groups[0].tabs, vec!["a", "a2", "b", "c"]);
        assert_eq!(g.active_tab(), Some(&"a2"));
        assert_eq!(g.find(&"c"), Some((0, 3)));
        assert_eq!(g.tabs().count(), 4);
    }

    /// One tall group on the left, two stacked on the right.
    fn tall_and_two() -> Layout {
        Layout {
            cols: vec![0., 0.5, 1.],
            rows: vec![0., 0.5, 1.],
            cells: vec![[0, 0, 1, 2], [1, 0, 2, 1], [1, 1, 2, 2]],
        }
    }

    #[test]
    fn splitting_makes_nested_layouts_whose_dividers_stop_at_groups() {
        let l = Layout::preset(Preset::Columns(2)).split(1, Axis::Rows).unwrap();
        assert_eq!(l, tall_and_two());
        assert!(l.is_valid());
        assert_eq!(
            l.dividers(),
            vec![d(Axis::Cols, 1, 0., 1.), d(Axis::Rows, 1, 0.5, 1.)],
            "the row line only on the right"
        );
        // the tall group split down uses the line already there, lined up with its neighbours
        let both = l.split(0, Axis::Rows).unwrap();
        assert_eq!(both.rows, vec![0., 0.5, 1.]);
        assert_eq!(both.cells, vec![[0, 0, 1, 1], [0, 1, 1, 2], [1, 0, 2, 1], [1, 1, 2, 2]]);
        assert_eq!(both.dividers().len(), 2, "now full lines: like Grid 4");
        // a group split right gets a new column line in its middle
        let three = tall_and_two().split(1, Axis::Cols).unwrap();
        assert_eq!(three.cols, vec![0., 0.5, 0.75, 1.]);
        assert_eq!(three.cells[0], [0, 0, 1, 2], "the tall one spans the new line");
        assert_eq!(three.cells[1..3], [[1, 0, 2, 1], [2, 0, 3, 1]]);
        assert_eq!(three.cells[3], [1, 1, 3, 2], "the one below spans it too");
        assert!(three.is_valid());
        assert!(Layout::default().split(5, Axis::Cols).is_none());
    }

    #[test]
    fn closing_a_group_grows_the_neighbours_that_cover_its_edge() {
        // the tall group goes: the two stacked ones grow left
        let (l, target) = tall_and_two().close(0).unwrap();
        assert_eq!(l, Layout::preset(Preset::Rows(2)));
        assert_eq!(target, 0);
        // a stacked one goes: the one above grows down
        let (l, target) = tall_and_two().close(2).unwrap();
        assert_eq!(l, Layout::preset(Preset::Columns(2)));
        assert_eq!(target, 1, "the one above grows: the tall one is taller than its edge");
        let (l, target) = Layout::preset(Preset::Columns(3)).close(1).unwrap();
        assert_eq!((l.cols.clone(), target), (vec![0., 0.66, 1.], 0));
        assert!(Layout::default().close(0).is_none(), "the only group stays");
        // a pinwheel has no side that fits: refused
        let pinwheel = Layout {
            cols: vec![0., 0.33, 0.66, 1.],
            rows: vec![0., 0.33, 0.66, 1.],
            cells: vec![[0, 0, 2, 1], [2, 0, 3, 2], [1, 2, 3, 3], [0, 1, 1, 3], [1, 1, 2, 2]],
        };
        assert!(pinwheel.is_valid());
        assert!(pinwheel.close(4).is_none());
        assert!(pinwheel.close(0).is_some());
    }

    #[test]
    fn a_layout_fits_an_area_when_every_group_is_big_enough() {
        let l = tall_and_two();
        assert!(l.fits(400., 300., 100., 100.));
        assert!(!l.fits(400., 150., 100., 100.), "the stacked ones are 75 high");
    }

    #[test]
    fn saved_layouts_round_trip_and_bad_entries_are_named() {
        let dir = std::env::temp_dir().join(format!("orlo-layouts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("Layouts.json");
        assert_eq!(load_saved(&p).unwrap(), (vec![], vec![]), "none yet");
        let mine = SavedLayout { name: "Tall and two".into(), layout: tall_and_two() };
        save_saved(&p, std::slice::from_ref(&mine)).unwrap();
        assert_eq!(load_saved(&p).unwrap(), (vec![mine.clone()], vec![]));
        std::fs::write(
            &p,
            r#"{"layouts": [
                {"name": "ok", "cols": [0, 1], "rows": [0, 1], "cells": [[0, 0, 1, 1]]},
                {"name": "gap", "cols": [0, 0.5, 1], "rows": [0, 1], "cells": [[0, 0, 1, 1]]},
                {"name": "", "cols": [0, 1], "rows": [0, 1], "cells": [[0, 0, 1, 1]]},
                {"name": "ok", "cols": [0, 1], "rows": [0, 1], "cells": [[0, 0, 1, 1]]},
                {"cols": 3},
            ]}"#,
        )
        .unwrap();
        let (ok, bad) = load_saved(&p).unwrap();
        assert_eq!(ok.len(), 1);
        assert_eq!(bad.len(), 4, "{bad:?}");
        std::fs::write(&p, "[").unwrap();
        assert!(load_saved(&p).is_err());
    }

    #[test]
    fn groups_split_and_close_keep_every_view_and_the_active_one() {
        let mut g = three_tabs();
        g.select(0, 1);
        let l = g.layout.split(0, Axis::Cols).unwrap();
        g.split(0, l);
        assert_eq!((g.groups.len(), g.active), (2, 1), "the new group has focus");
        g.add("d");
        g.add("e");
        g.select(1, 0);
        // closing the new group: its views go to the one that grows, d stays active
        let (l, target) = g.layout.close(1).unwrap();
        g.close(1, l, target);
        assert_eq!(g.groups.len(), 1);
        assert_eq!(g.groups[0].tabs, vec!["a", "b", "c", "d", "e"]);
        assert_eq!(g.active_tab(), Some(&"d"));
        assert_eq!(g.layout, Layout::default());
    }

    #[test]
    fn multi_selected_tabs_show_together_and_clicks_and_closes_move_the_input() {
        let mut g = three_tabs();
        g.add("d");
        g.select(0, 0);
        g.toggle_selected(0, 2);
        assert_eq!(g.shown(0), vec!["a", "c"]);
        assert_eq!(g.active_tab(), Some(&"c"), "the ⌘-clicked tab has the input");
        // a click into one shown keeps them all; a tab not shown ends it
        g.select(0, 0);
        assert_eq!((g.shown(0), g.active_tab()), (vec!["a", "c"], Some(&"a")));
        g.select(0, 1);
        assert_eq!(g.shown(0), vec!["b"]);
        // ⇧-click: the range; closing the one with the input moves it to another shown
        g.select_range(0, 3);
        assert_eq!(g.shown(0), vec!["b", "c", "d"]);
        g.remove(&"d");
        assert_eq!(g.shown(0), vec!["b", "c"]);
        assert!(g.active_tab() == Some(&"b") || g.active_tab() == Some(&"c"));
        // ⌘-click on the input tab takes it out; on another shown one too
        let input = *g.active_tab().unwrap();
        let idx = g.groups[0].tabs.iter().position(|t| *t == input).unwrap();
        g.toggle_selected(0, idx);
        assert_eq!(g.shown(0).len(), 1);
        assert_ne!(g.active_tab(), Some(&input));
    }

    #[test]
    fn moving_tabs_by_identity_keeps_pinned_ones_first() {
        let mut g = three_tabs();
        g.set_layout(Layout::preset(Preset::Columns(2)));
        g.select(0, 0);
        // reorder: a goes after c
        g.move_tab(&"a", 0, 3);
        assert_eq!(g.groups[0].tabs, vec!["b", "c", "a"]);
        assert_eq!(g.active_tab(), Some(&"a"));
        // pin c: first; an unpinned tab cannot go in front of it
        g.set_pinned(&"c", true);
        assert_eq!(g.groups[0].tabs, vec!["c", "b", "a"]);
        g.move_tab(&"a", 0, 0);
        assert_eq!(g.groups[0].tabs, vec!["c", "a", "b"], "behind the pinned one");
        // to another group, into an empty one; a pinned one stays pinned there
        g.move_tab(&"c", 1, 0);
        assert_eq!((g.groups[1].tabs.clone(), g.active), (vec!["c"], 1));
        assert!(g.is_pinned(&"c"));
        assert_eq!(g.groups[0].pinned, Vec::<&str>::new());
        assert_eq!(g.move_tab(&"gone", 0, 0), None, "a stale tab moves nothing");
        assert_eq!(g.move_tab(&"a", 9, 0), None);
        g.set_pinned(&"c", false);
        assert!(!g.is_pinned(&"c"));
        // a preview stops being one when moved or pinned
        g.groups[0].preview = Some("b");
        g.move_tab(&"b", 1, 5);
        assert!(!g.is_preview(&"b"));
        assert_eq!(g.groups[1].tabs, vec!["c", "b"]);
    }
}
