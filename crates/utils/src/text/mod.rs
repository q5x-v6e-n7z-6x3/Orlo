//! Pure rope and line difference algorithms.

use ropey::Rope;
use std::cmp::Ordering;

/// ASCII digit runs have arbitrary precision; equal values prefer fewer leading zeros.
pub fn natural_cmp(mut a: &[u8], mut b: &[u8]) -> Ordering {
    let mut zeros = Ordering::Equal;
    while let (Some(&ac), Some(&bc)) = (a.first(), b.first()) {
        if ac.is_ascii_digit() && bc.is_ascii_digit() {
            let al = a.iter().take_while(|c| c.is_ascii_digit()).count();
            let bl = b.iter().take_while(|c| c.is_ascii_digit()).count();
            let an = &a[..al];
            let bn = &b[..bl];
            let an = &an[an.iter().take_while(|&&c| c == b'0').count()..];
            let bn = &bn[bn.iter().take_while(|&&c| c == b'0').count()..];
            let cmp = an.len().cmp(&bn.len()).then_with(|| an.cmp(bn));
            if cmp != Ordering::Equal {
                return cmp;
            }
            zeros = zeros.then_with(|| al.cmp(&bl));
            a = &a[al..];
            b = &b[bl..];
        } else {
            match ac.cmp(&bc) {
                Ordering::Equal => {
                    a = &a[1..];
                    b = &b[1..];
                }
                cmp => return cmp,
            }
        }
    }
    a.len().cmp(&b.len()).then(zeros)
}

/// One rope edit, in chars: `removed` chars at `start` replaced by `inserted` chars. A list of
/// them applies in order, each in the coordinates the previous ones left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub start: usize,
    pub removed: usize,
    pub inserted: usize,
    /// Line breaks among the removed and the inserted chars: a wrap map re-lines by them.
    pub lines: (usize, usize),
}

/// The changes that turn `a` into `b`: the middle between their common prefix and suffix, split
/// further by `minimal`.
pub fn diff(a: &Rope, b: &Rope) -> Vec<Change> {
    let (la, lb) = (a.len_chars(), b.len_chars());
    let max = la.min(lb);
    // whole chunks compared at once: char by char, undo in a 100k-line file takes 28.8 ms
    let start = a.byte_to_char(common_bytes(a.chunks(), b.chunks(), |c| c.next(), false));
    let (ra, rb) = (a.chunks_at_byte(a.len_bytes()).0, b.chunks_at_byte(b.len_bytes()).0);
    let tail = a.len_bytes() - common_bytes(ra, rb, |c| c.prev(), true);
    // a char only partly in the common bytes is not common
    let first = a.byte_to_char(tail) + usize::from(a.char_to_byte(a.byte_to_char(tail)) < tail);
    let end = (la - first).min(max - start);
    let mut out = Vec::new();
    minimal(start, &a.slice(start..la - end).to_string(), &b.slice(start..lb - end).to_string(), &mut out);
    out
}

/// How many bytes two ropes' chunk streams share from their start (`back`: from their end).
pub fn common_bytes<'a, I>(a: I, b: I, next: impl Fn(&mut I) -> Option<&'a str>, back: bool) -> usize {
    common_bytes_up_to(a, b, next, back, usize::MAX)
}

/// Compare at most `max` bytes, skipping chunks shared by both ropes.
pub fn common_bytes_up_to<'a, I>(
    mut a: I,
    mut b: I,
    next: impl Fn(&mut I) -> Option<&'a str>,
    back: bool,
    max: usize,
) -> usize {
    let (mut x, mut y): (&[u8], &[u8]) = (&[], &[]);
    let mut n = 0;
    while n < max {
        if x.is_empty() {
            match next(&mut a) {
                Some(c) => x = c.as_bytes(),
                None => return n,
            }
        }
        if y.is_empty() {
            match next(&mut b) {
                Some(c) => y = c.as_bytes(),
                None => return n,
            }
        }
        let k = x.len().min(y.len()).min(max - n);
        let (xs, ys) = match back {
            false => (&x[..k], &y[..k]),
            true => (&x[x.len() - k..], &y[y.len() - k..]),
        };
        if xs.as_ptr() != ys.as_ptr() && xs != ys {
            let same = match back {
                false => xs.iter().zip(ys).take_while(|(p, q)| p == q).count(),
                true => xs.iter().rev().zip(ys.iter().rev()).take_while(|(p, q)| p == q).count(),
            };
            return n + same;
        }
        n += k;
        (x, y) = match back {
            false => (&x[k..], &y[k..]),
            true => (&x[..x.len() - k], &y[..y.len() - k]),
        };
    }
    n
}

/// The changes that turn `old` (at `start`) into `new`, small enough that carets and
/// bookmarks stay on their characters: gap by gap when only the spacing between words changed
/// (wrap, join; a word may move to another line), line by line when the line count holds
/// (indent, comment, sort, multi-caret undo), hunk by hunk around the lines both share (a
/// reload), else the middle between the common prefix and suffix.
pub fn minimal(start: usize, old: &str, new: &str, out: &mut Vec<Change>) {
    // a pure insertion or deletion has nothing to line up
    if old.is_empty() || new.is_empty() {
        return out.extend(trim(start, old, new));
    }
    let lines = old.matches('\n').count();
    let same_lines = lines > 0 && lines == new.matches('\n').count();
    // the words first, a cheap look that stops at the first change: the runs line up only when
    // the words are the same (a sort, an indent or a paste changes them)
    fn words(s: &str) -> impl Iterator<Item = char> + '_ {
        s.chars().filter(|c| !c.is_whitespace())
    }
    if words(old).eq(words(new)) && by_runs(start, old, new, out) {
        return;
    }
    if same_lines {
        return pairwise(start, old.split('\n'), new.split('\n'), 1, out);
    }
    if (old.contains('\n') || new.contains('\n')) && by_lines(start, old, new, out) {
        return;
    }
    out.extend(trim(start, old, new));
}

/// The runs of `old` and `new` (`runs`) paired, when they line up: the same count, and each the
/// same or both spacing. Taken as they are found (a whole-file indent has tens of millions);
/// false, with nothing added, when they do not line up after all.
pub fn by_runs(mut pos: usize, old: &str, new: &str, out: &mut Vec<Change>) -> bool {
    let blank = |s: &str| s.starts_with(char::is_whitespace);
    let (mut ow, mut nw, kept) = (runs(old), runs(new), out.len());
    loop {
        match (ow.next(), nw.next()) {
            (None, None) => return true,
            (Some(o), Some(n)) if o == n || (blank(o) && blank(n)) => {
                if o != n {
                    out.extend(trim(pos, o, n));
                }
                pos += n.chars().count();
            }
            _ => {
                out.truncate(kept);
                return false;
            }
        }
    }
}

/// The hunks between the lines `old` and `new` have in common, each trimmed (line by line when
/// it keeps its line count); false when they share no line.
pub fn by_lines(mut pos: usize, old: &str, new: &str, out: &mut Vec<Change>) -> bool {
    let a: Vec<&str> = old.split_inclusive('\n').collect();
    let b: Vec<&str> = new.split_inclusive('\n').collect();
    let pairs = match common_lines(&a, &b) {
        Some(p) if !p.is_empty() => p,
        _ => return false,
    };
    let chars = |ls: &[&str]| ls.iter().map(|l| l.chars().count()).sum::<usize>();
    let (mut i, mut j) = (0, 0);
    for (mi, mj) in pairs.into_iter().chain([(a.len(), b.len())]) {
        let (o, n) = (&a[i..mi], &b[j..mj]);
        if o.len() == n.len() {
            pairwise(pos, o.iter().copied(), n.iter().copied(), 0, out);
        } else {
            out.extend(trim(pos, &o.concat(), &n.concat()));
        }
        pos += chars(n) + b.get(mj).map_or(0, |l| l.chars().count());
        (i, j) = (mi + 1, mj + 1);
    }
    true
}

/// The equal lines of `a` and `b` as (index in a, index in b), by Myers' diff.
/// Gives up past 500 line edits (a file rewritten wholesale), where one change does.
pub fn common_lines(a: &[&str], b: &[&str]) -> Option<Vec<(usize, usize)>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (n + m).min(500);
    let off = max + 1;
    let mut v = vec![0isize; 2 * off as usize + 1];
    // trace[d]: the furthest x on each diagonal before step d
    let mut trace = Vec::new();
    for d in 0..=max {
        trace.push(v.clone());
        for k in (-d..=d).step_by(2) {
            let down = k == -d || (k != d && v[(off + k - 1) as usize] < v[(off + k + 1) as usize]);
            let mut x = if down { v[(off + k + 1) as usize] } else { v[(off + k - 1) as usize] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                (x, y) = (x + 1, y + 1);
            }
            v[(off + k) as usize] = x;
            if x >= n && y >= m {
                return Some(backtrack(&trace, off, n, m));
            }
        }
    }
    None
}

/// Walk Myers' trace back from the end, collecting the diagonal (equal) steps.
pub fn backtrack(trace: &[Vec<isize>], off: isize, n: isize, m: isize) -> Vec<(usize, usize)> {
    let (mut x, mut y) = (n, m);
    let mut pairs = Vec::new();
    for (d, v) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let k = x - y;
        let (px, py) = if d == 0 {
            (0, 0)
        } else {
            let down = k == -d || (k != d && v[(off + k - 1) as usize] < v[(off + k + 1) as usize]);
            let pk = if down { k + 1 } else { k - 1 };
            let px = v[(off + pk) as usize];
            (px, px - pk)
        };
        while x > px && y > py {
            (x, y) = (x - 1, y - 1);
            pairs.push((x as usize, y as usize));
        }
        (x, y) = (px, py);
    }
    pairs.reverse();
    pairs
}

/// Pieces of `old` and `new` paired in order, `sep` chars between pieces (equal on both sides).
pub fn pairwise<'a>(
    mut pos: usize,
    old: impl Iterator<Item = &'a str>,
    new: impl Iterator<Item = &'a str>,
    sep: usize,
    out: &mut Vec<Change>,
) {
    for (o, n) in old.zip(new) {
        // most lines are the same: one memcmp, not trim's char by char walks
        if o != n {
            out.extend(trim(pos, o, n));
        }
        pos += n.chars().count() + sep;
    }
}

/// `old` replaced by `new` at `start`, minus their common prefix and suffix; `None` when equal.
pub fn trim(start: usize, old: &str, new: &str) -> Option<Change> {
    let (lo, ln) = (old.chars().count(), new.chars().count());
    let pre = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();
    let suf = old.chars().rev().zip(new.chars().rev()).take(lo.min(ln) - pre).take_while(|(a, b)| a == b).count();
    let breaks = |s: &str, n: usize| s.chars().skip(pre).take(n).filter(|&c| c == '\n').count();
    (pre < lo.max(ln)).then(|| Change {
        start: start + pre,
        removed: lo - pre - suf,
        inserted: ln - pre - suf,
        lines: (breaks(old, lo - pre - suf), breaks(new, ln - pre - suf)),
    })
}

/// `s` cut into alternating runs of whitespace and non-whitespace.
pub fn runs(s: &str) -> impl Iterator<Item = &str> {
    let mut rest = s;
    std::iter::from_fn(move || {
        let ws = rest.chars().next()?.is_whitespace();
        let end = rest.find(|c: char| c.is_whitespace() != ws).unwrap_or(rest.len());
        let (run, tail) = rest.split_at(end);
        rest = tail;
        Some(run)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_chunk_comparison_matches_bytes_and_stops_at_the_limit() {
        for a in ["", "éabc", "abc猫", "x".repeat(3000).as_str()] {
            for b in ["", "éabc", "xyabc", "abc猫"] {
                let (a, b) = (Rope::from_str(a), Rope::from_str(b));
                let (ab, bb) = (a.to_string(), b.to_string());
                for max in [0, 1, 2, 4, 5000] {
                    let prefix = ab.bytes().zip(bb.bytes()).take_while(|(a, b)| a == b).count();
                    let suffix = ab.bytes().rev().zip(bb.bytes().rev()).take_while(|(a, b)| a == b).count();
                    assert_eq!(common_bytes_up_to(a.chunks(), b.chunks(), |c| c.next(), false, max), prefix.min(max));
                    assert_eq!(
                        common_bytes_up_to(
                            a.chunks_at_byte(a.len_bytes()).0,
                            b.chunks_at_byte(b.len_bytes()).0,
                            |c| c.prev(),
                            true,
                            max
                        ),
                        suffix.min(max)
                    );
                }
            }
        }
        let calls = std::cell::Cell::new(0);
        let chunks = ["abc", "def", "ghi"].into_iter();
        assert_eq!(
            common_bytes_up_to(
                chunks.clone(),
                chunks,
                |c| {
                    calls.set(calls.get() + 1);
                    c.next()
                },
                false,
                2
            ),
            2
        );
        assert_eq!(calls.get(), 2, "no chunks beyond the bound are requested");
    }
}
