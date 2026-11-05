use super::*;
pub use orlo_utils::text::{common_lines, diff, minimal, Change};

/// `ranges` (sorted, disjoint) after `changes`, each keeping only its own surviving text: text
/// typed at either end, or replacing chars it covered, stays outside; the ones an edit swallowed
/// are dropped. Starts and ends are each a sorted list, mapped in one sweep.
pub fn map_ranges(ranges: &mut Vec<Range<usize>>, changes: &[Change]) {
    let empty: Vec<bool> = ranges.iter().map(|r| r.is_empty()).collect();
    let mut starts: Vec<usize> = ranges.iter().map(|r| r.start).collect();
    let mut ends: Vec<usize> = ranges.iter().map(|r| r.end).collect();
    map_sorted_starts(&mut starts, changes);
    map_sorted(&mut ends, changes);
    *ranges = (starts.into_iter().zip(ends).zip(empty))
        // Zero-width results move as carets; an erased non-empty match still disappears.
        .filter_map(|((s, e), empty)| if empty { Some(e..e) } else { (s < e).then_some(s..e) })
        .collect();
}

/// Where `offset` ends up after `changes`: shifted when after a change, moved to the start of a
/// change that removed it.
pub fn map_offset(mut offset: usize, changes: &[Change]) -> usize {
    for c in changes {
        if offset <= c.start {
            continue;
        }
        offset = if offset < c.start + c.removed { c.start } else { offset - c.removed + c.inserted };
    }
    offset
}

/// `map_offset`, but an offset inside a change keeps as far in, within what replaced it
/// (exact when as long as it was): a selection in a word an overlapping Upper Case remade
/// stays where it was.
pub(crate) fn map_within(mut offset: usize, changes: &[Change]) -> usize {
    for c in changes {
        if offset <= c.start {
            continue;
        }
        offset = match offset < c.start + c.removed {
            true => c.start + (offset - c.start).min(c.inserted),
            false => offset - c.removed + c.inserted,
        };
    }
    offset
}

/// `map_offset` for an offset in the original text through sorted, disjoint `groups` that
/// were applied as `applied`.
pub(crate) fn map_offset_sorted(p: usize, groups: &[(Range<usize>, String)], applied: &[Change]) -> usize {
    // the last group starting before p
    let k = groups.partition_point(|(r, _)| r.start < p);
    if k == 0 {
        return p;
    }
    let (r, c) = (&groups[k - 1].0, &applied[k - 1]);
    if p < r.end {
        // inside it: as far in, within what replaced it (exact when as long as it was)
        c.start + (p - r.start).min(c.inserted)
    } else {
        p - r.end + c.start + c.inserted
    }
}

/// `map_offset` for every offset in `xs` (sorted), sweeping each in-order run of `changes` once
/// instead of every change per offset: a sibling with thousands of carets follows a Sort Lines
/// or Replace All of thousands of changes in one pass.
pub fn map_sorted(xs: &mut [usize], changes: &[Change]) {
    sweep(xs, changes, false);
}

/// `map_sorted` for the starts of ranges: an offset at an insertion, or inside text a change
/// replaced, goes after the inserted text, where its first surviving char is. Exact within a run
/// of changes, as the editor logs an edit; across runs (rare), text an earlier run inserted
/// counts as text that was there.
pub fn map_sorted_starts(xs: &mut [usize], changes: &[Change]) {
    sweep(xs, changes, true);
}

pub(crate) fn push_state(stack: &mut Vec<SelState>, state: SelState) {
    if stack.len() == SEL_HISTORY {
        stack.remove(0);
    }
    stack.push(state);
}

/// Keep `sels` on their characters across `changes`; with `after`, an offset where text was
/// inserted goes after it.
pub(crate) fn map_sels(sels: &mut [Selection], changes: &[Change], after: bool) {
    // An edit at a virtual endpoint invalidates its gap; edits elsewhere keep it.
    for s in sels.iter_mut().filter(|s| s.has_virtual()) {
        for (mut offset, gap) in [(s.anchor, &mut s.anchor_virtual), (s.head, &mut s.head_virtual)] {
            for c in changes {
                if c.start <= offset && offset <= c.start + c.removed {
                    *gap = 0;
                }
                offset = map_offset(offset, std::slice::from_ref(c));
            }
        }
    }
    let mut pts: Vec<(usize, usize)> =
        sels.iter().flat_map(|s| [s.anchor, s.head]).enumerate().map(|(i, o)| (o, i)).collect();
    pts.sort_unstable();
    let mut xs: Vec<usize> = pts.iter().map(|p| p.0).collect();
    match after {
        true => map_sorted_starts(&mut xs, changes),
        false => map_sorted(&mut xs, changes),
    }
    for ((_, i), x) in pts.into_iter().zip(xs) {
        let s = &mut sels[i / 2];
        *if i % 2 == 0 { &mut s.anchor } else { &mut s.head } = x;
    }
}

/// `changes` cut into runs, each given in the coordinates before it: ascending and disjoint.
/// Ascending runs (typing, diffs) start after the text the previous change inserted;
/// descending runs (replace all, line commands) end before the previous change starts, and
/// an insertion right at its start would not push what that change collapsed there.
pub fn change_runs(changes: &[Change]) -> impl Iterator<Item = Vec<Change>> + '_ {
    let up = |a: &Change, b: &Change| b.start >= a.start + a.inserted;
    let down = |a: &Change, b: &Change| b.start + b.removed <= a.start && (b.removed > 0 || b.start < a.start);
    let run_len = |cs: &[Change], ok: &dyn Fn(&Change, &Change) -> bool| {
        1 + cs.windows(2).take_while(|w| ok(&w[0], &w[1])).count()
    };
    let mut rest = changes;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let (n_up, n_down) = (run_len(rest, &up), run_len(rest, &down));
        let (run, tail) = rest.split_at(n_up.max(n_down));
        rest = tail;
        if n_up < n_down {
            return Some(run.iter().rev().copied().collect());
        }
        let mut d = 0isize;
        Some(
            (run.iter())
                .map(|c| {
                    let start = (c.start as isize - d) as usize;
                    d += c.inserted as isize - c.removed as isize;
                    Change { start, ..*c }
                })
                .collect(),
        )
    })
}

/// Bookmarks (sorted line starts) through `changes`: past a change that ends at or before one,
/// it shifts on (an insertion at it pushes it on with its line); inside removed text it moves
/// to the change's start. One sweep per in-order run, not every change for every bookmark.
pub(crate) fn map_bookmarks(xs: &mut [usize], changes: &[Change]) {
    if xs.is_empty() {
        return;
    }
    for run in change_runs(changes) {
        let (mut k, mut d) = (0, 0isize);
        for x in xs.iter_mut() {
            while k < run.len() && *x >= run[k].start + run[k].removed {
                d += run[k].inserted as isize - run[k].removed as isize;
                k += 1;
            }
            let at = if k < run.len() && *x > run[k].start { run[k].start } else { *x };
            *x = (at as isize + d) as usize;
        }
    }
}

pub(crate) fn sweep(xs: &mut [usize], changes: &[Change], after: bool) {
    for run in change_runs(changes) {
        let (mut k, mut d) = (0, 0isize);
        // past change k: after its removed text; a start also sits past an insertion at it
        let past = |x: usize, c: &Change| x >= c.start + c.removed && (after || x > c.start);
        let inside = |x: usize, c: &Change| x > c.start || after && x == c.start;
        for x in xs.iter_mut() {
            while k < run.len() && past(*x, &run[k]) {
                d += run[k].inserted as isize - run[k].removed as isize;
                k += 1;
            }
            // a start in removed text: its first surviving char is past that text, and past a
            // change right after it too (on a cursor of its own: the next x may sit before it)
            let (mut kk, mut dd, mut y) = (k, d, *x);
            while after && kk < run.len() && inside(y, &run[kk]) {
                y = run[kk].start + run[kk].removed;
                while kk < run.len() && past(y, &run[kk]) {
                    dd += run[kk].inserted as isize - run[kk].removed as isize;
                    kk += 1;
                }
            }
            let at = match kk < run.len() && inside(y, &run[kk]) {
                true => run[kk].start,
                false => y,
            };
            *x = (at as isize + dd) as usize;
        }
    }
}
