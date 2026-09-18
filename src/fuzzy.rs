//! A small, dependency-free fuzzy matcher for picking a kubeconfig
//! context by a partial/typo-tolerant name — used both by `--context`'s
//! one-shot resolution and by the interactive cluster picker's
//! type-to-filter search.

/// Case-insensitive subsequence match: every character of `pattern` must
/// appear in `text` in order, though not necessarily contiguously.
/// Returns `None` when it doesn't match at all; otherwise a score where
/// higher means "reads more like what the user meant" — rewarding matches
/// that start at a word boundary and runs of consecutive characters over
/// characters scattered arbitrarily far apart. An empty pattern always
/// matches everything, with a score of 0 (so an empty filter shows every
/// candidate, unranked).
pub fn score(pattern: &str, text: &str) -> Option<i64> {
    if pattern.is_empty() {
        return Some(0);
    }

    let text_chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut ti = 0;
    let mut total = 0i64;
    let mut prev_matched: Option<usize> = None;

    for pc in pattern.to_lowercase().chars() {
        let idx = (ti..text_chars.len()).find(|&i| text_chars[i] == pc)?;
        total += 1;
        if idx == 0 {
            total += 8;
        } else if matches!(text_chars[idx - 1], '-' | '_' | '.' | '/' | ':') {
            total += 5;
        }
        if prev_matched == Some(idx.wrapping_sub(1)) {
            total += 3;
        }
        prev_matched = Some(idx);
        ti = idx + 1;
    }
    Some(total)
}

/// The single best-scoring candidate, or `None` if nothing matches.
pub fn best_match<'a>(pattern: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    candidates.filter_map(|c| score(pattern, c).map(|s| (s, c))).max_by_key(|(s, _)| *s).map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pattern_matches_everything() {
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn out_of_order_characters_do_not_match() {
        assert_eq!(score("ba", "ab"), None);
    }

    #[test]
    fn scattered_subsequence_still_matches() {
        assert!(score("pd", "prod").is_some());
    }

    #[test]
    fn prefix_match_scores_higher_than_a_scattered_one() {
        let prefix = score("prod", "prod-us-east").unwrap();
        let scattered = score("prod", "sparodic").unwrap();
        assert!(prefix > scattered);
    }

    #[test]
    fn best_match_picks_the_highest_scoring_candidate() {
        let candidates = ["staging-eu", "unproductive", "production-us"];
        assert_eq!(best_match("prod", candidates.into_iter()), Some("production-us"));
    }

    #[test]
    fn best_match_is_none_when_nothing_matches() {
        let candidates = ["staging", "production"];
        assert_eq!(best_match("xyz", candidates.into_iter()), None);
    }
}
