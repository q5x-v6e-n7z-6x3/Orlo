//! Line changes between a saved or indexed baseline and the current buffer.

use ropey::Rope;
use std::ops::Range;

/// Zero-based, half-open ranges. Character ranges include the changed line terminators.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_chars: Range<usize>,
    pub new_chars: Range<usize>,
    pub old_lines: Range<usize>,
    pub new_lines: Range<usize>,
}

pub fn between(base: &Rope, current: &Rope) -> Vec<Hunk> {
    let (old, new) = (base.to_string(), current.to_string());
    let a: Vec<&str> = old.split_inclusive('\n').collect();
    let b: Vec<&str> = new.split_inclusive('\n').collect();
    let offsets = |lines: &[&str]| {
        std::iter::once(0)
            .chain(lines.iter().scan(0, |offset, line| {
                *offset += line.chars().count();
                Some(*offset)
            }))
            .collect::<Vec<_>>()
    };
    let (ac, bc) = (offsets(&a), offsets(&b));
    let prefix = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    let (ae, be) = (a.len() - suffix, b.len() - suffix);
    // over 500 line edits use one middle hunk; raise the bound if rewrite granularity matters.
    let pairs = orlo_utils::text::common_lines(&a[prefix..ae], &b[prefix..be]).unwrap_or_default();
    let (mut i, mut j) = (prefix, prefix);
    let mut hunks = Vec::new();
    for (mi, mj) in pairs.into_iter().map(|(i, j)| (prefix + i, prefix + j)).chain([(ae, be)]) {
        if i < mi || j < mj {
            hunks.push(Hunk { old_chars: ac[i]..ac[mi], new_chars: bc[j]..bc[mj], old_lines: i..mi, new_lines: j..mj });
        }
        (i, j) = (mi + 1, mj + 1);
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(old: &str, new: &str) -> Vec<Hunk> {
        between(&Rope::from_str(old), &Rope::from_str(new))
    }

    /// A hunk from its old and new character ranges, then its old and new line ranges.
    fn hunk(chars: [Range<usize>; 2], lines: [Range<usize>; 2]) -> Hunk {
        let ([old_chars, new_chars], [old_lines, new_lines]) = (chars, lines);
        Hunk { old_chars, new_chars, old_lines, new_lines }
    }

    #[test]
    fn additions_deletions_and_replacements_stay_separate() {
        let hunks = diff("a\nold\nb\ndrop\nc\n", "a\nnew\nb\nc\nextra\n");
        assert_eq!(
            hunks,
            [hunk([2..6, 2..6], [1..2, 1..2]), hunk([8..13, 8..8], [3..4, 3..3]), hunk([15..15, 10..16], [5..5, 4..5]),]
        );
    }

    #[test]
    fn empty_and_eof_changes_keep_their_text() {
        assert!(diff("", "").is_empty());
        assert!(diff("same\n", "same\n").is_empty());
        assert_eq!(diff("", "猫"), [hunk([0..0, 0..1], [0..0, 0..1])]);
        assert_eq!(diff("a\n猫", "a\n"), [hunk([2..3, 2..2], [1..2, 1..1])]);
        assert_eq!(diff("猫", ""), [hunk([0..1, 0..0], [0..1, 0..0])]);
        assert_eq!(diff("a\n", "a"), [hunk([0..2, 0..1], [0..1, 0..1])]);
    }

    #[test]
    fn repeated_lines_do_not_swallow_the_unchanged_middle() {
        let hunks = diff("x\na\nx\nb\nx\n", "x\nA\nx\nB\nx\n");
        assert_eq!(hunks.len(), 2);
        assert_eq!((hunks[0].old_lines.clone(), hunks[0].new_lines.clone()), (1..2, 1..2));
        assert_eq!((hunks[1].old_lines.clone(), hunks[1].new_lines.clone()), (3..4, 3..4));
    }

    #[test]
    fn unicode_and_crlf_are_counted_as_characters_and_retained() {
        let old = Rope::from_str("é\r\n猫\r\nlast");
        let new = Rope::from_str("é\r\n🐈\r\nlast");
        let hunks = between(&old, &new);
        assert_eq!(hunks, [hunk([3..6, 3..6], [1..2, 1..2])]);
        assert_eq!(old.slice(hunks[0].old_chars.clone()).to_string(), "猫\r\n");
        assert_eq!(new.slice(hunks[0].new_chars.clone()).to_string(), "🐈\r\n");
    }

    #[test]
    fn a_large_rewrite_falls_back_to_one_hunk_between_unchanged_edges() {
        let old = format!("top\n{}end\n", "old\n".repeat(5_000));
        let new = format!("top\n{}end\n", "new\n".repeat(5_000));
        assert_eq!(diff(&old, &new), [hunk([4..20_004, 4..20_004], [1..5_001, 1..5_001])]);
    }

    #[test]
    fn hunk_text_can_apply_and_revert_each_change_without_losing_terminators() {
        let texts = ["", "a", "a\n", "a\n\n", "a\nb\na\n", "b\na\nb", "é\r\n猫\r\n", "🐈\nlast"];
        for old in texts {
            for new in texts {
                let (base, current) = (Rope::from_str(old), Rope::from_str(new));
                let hunks = between(&base, &current);
                let (mut applied, mut reverted) = (base.clone(), current.clone());
                for hunk in hunks.iter().rev() {
                    applied.remove(hunk.old_chars.clone());
                    applied.insert(hunk.old_chars.start, &current.slice(hunk.new_chars.clone()).to_string());
                    reverted.remove(hunk.new_chars.clone());
                    reverted.insert(hunk.new_chars.start, &base.slice(hunk.old_chars.clone()).to_string());
                }
                assert_eq!(applied.to_string(), new, "applying {old:?} -> {new:?}");
                assert_eq!(reverted.to_string(), old, "reverting {old:?} -> {new:?}");
            }
        }
    }
}
