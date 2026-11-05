//! Subsequence fuzzy matching for the quick panels.

/// `c` compared without case: ASCII fast, other letters by their lowercase (É matches é;
/// accents still count, as in Sublime).
fn fold(c: char) -> char {
    match c.is_ascii() {
        true => c.to_ascii_lowercase(),
        false => c.to_lowercase().next().unwrap_or(c),
    }
}

/// Sublime's query: space-separated terms, each matched on its own, in any order.
fn terms(query: &str) -> impl Iterator<Item = &str> {
    query.split_whitespace()
}

/// Every term of `query` as a subsequence of `candidate`, scored and summed. Higher is better;
/// `None` means no match.
pub fn score(query: &str, candidate: &str) -> Option<i32> {
    terms(query).map(|t| score_term(t, candidate)).sum()
}

/// Greedy subsequence match of one term.
/// Greedy alignment can miss a better one; upgrade to Smith-Waterman if ranking looks off.
fn score_term(term: &str, candidate: &str) -> Option<i32> {
    let mut query = term.chars().map(fold).peekable();
    let mut score = 0i32;
    let mut last: Option<usize> = None;
    let mut previous = '\0';
    let mut chars = candidate.chars().enumerate();
    while let Some((i, ch)) = chars.next() {
        if query.peek() == Some(&fold(ch)) {
            score += 10;
            match last {
                Some(l) if l + 1 == i => score += 15,
                Some(l) => score -= (i - l - 1) as i32,
                None => {}
            }
            let word_start = i == 0
                || matches!(previous, '/' | '\\' | '_' | '-' | '.' | ' ')
                || (ch.is_uppercase() && previous.is_lowercase());
            if word_start {
                score += 10;
            }
            last = Some(i);
            query.next();
            if query.peek().is_none() {
                score -= (chars.count() / 10) as i32;
                return Some(score);
            }
        }
        previous = ch;
    }
    None
}

/// Byte ranges of the chars `score` matched, for drawing them highlighted; empty when the
/// query is empty or does not match.
pub fn positions(query: &str, candidate: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    for t in terms(query) {
        let mut q = t.chars().map(fold).peekable();
        for (b, ch) in candidate.char_indices() {
            if q.peek() == Some(&fold(ch)) {
                q.next();
                out.push(b..b + ch.len_utf8());
            }
        }
        if q.peek().is_some() {
            return Vec::new();
        }
    }
    out.sort_by_key(|r| r.start);
    out.dedup();
    out
}

/// Indices of matching candidates, best first; ties keep input order.
pub fn filter(query: &str, candidates: &[String]) -> Vec<usize> {
    if terms(query).next().is_none() {
        return (0..candidates.len()).collect();
    }
    let mut scored: Vec<(i32, usize)> =
        candidates.iter().enumerate().filter_map(|(i, c)| score(query, c).map(|s| (s, i))).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_separate_terms_in_any_order_and_case_folds_beyond_ascii() {
        // Sublime 4215
        assert!(score("ba fo", "foo_bar.rs").is_some());
        assert!(score("fo ba", "foo_bar.rs").is_some());
        assert!(score("fo zz", "foo_bar.rs").is_none());
        assert!(score("ÉCOLE", "école.txt").is_some());
        assert!(score("Ö", "ök").is_some());
        assert!(score("etu", "étude").is_none(), "accents still count");
        assert_eq!(positions("ba fo", "foo_bar"), vec![0..1, 1..2, 4..5, 5..6]);
    }

    #[test]
    fn rejects_non_subsequence() {
        assert!(score("abc", "acb").is_none());
    }

    #[test]
    fn scores_keep_unicode_boundaries_gaps_and_trailing_character_penalties() {
        assert_eq!(score("fb", "foo_bar.rs"), Some(37));
        assert_eq!(score("ba fo", "foo_bar.rs"), Some(90));
        assert_eq!(score("éx", "é_😀X"), Some(28));
        assert_eq!(score("É a", "éA____"), Some(40));
        assert_eq!(score("a", &format!("a{}", "😀".repeat(20))), Some(18));
        assert_eq!(score("", ""), Some(0));
        assert_eq!(score("x", ""), None);
    }

    #[test]
    fn prefers_word_starts_and_consecutive() {
        assert!(score("wsp", "workspace.rs") > score("wsp", "w_long_gap_s_more_p.rs"));
        assert!(score("text", "src/text.rs") > score("text", "t_e_x_t_long.rs"));
    }

    #[test]
    fn positions_are_the_greedy_matches_as_byte_ranges() {
        assert_eq!(positions("sel", "Set Syntax: Lua"), vec![0..1, 1..2, 12..13]);
        assert_eq!(positions("éa", "xéya"), vec![1..3, 4..5]);
        assert!(positions("zz", "abc").is_empty());
        assert!(positions("", "abc").is_empty());
    }

    #[test]
    fn filter_orders_best_first_and_empty_query_keeps_order() {
        let c = vec!["b.rs".to_string(), "a.rs".to_string(), "zzz".to_string()];
        assert_eq!(filter("rs", &c), vec![0, 1]);
        assert_eq!(filter("", &c), vec![0, 1, 2]);
        assert_eq!(filter(" \t\n\u{2003}", &c), vec![0, 1, 2]);
        assert!(filter("", &[]).is_empty());
    }
}
