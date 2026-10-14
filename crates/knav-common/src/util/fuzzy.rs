//! Fuzzy matching: the query's characters in order, not necessarily adjacent.

/// Case-insensitive subsequence match. `None` if it doesn't match, else a score
/// that rewards word starts and consecutive runs. An empty pattern scores 0.
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

/// Where `score` matched each character of `pattern`, for highlighting.
/// `None` when it doesn't match; empty for an empty pattern.
pub fn positions(pattern: &str, text: &str) -> Option<Vec<usize>> {
    let text_chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut at = 0;
    let mut found = Vec::new();
    for pc in pattern.to_lowercase().chars() {
        let idx = (at..text_chars.len()).find(|&i| text_chars[i] == pc)?;
        found.push(idx);
        at = idx + 1;
    }
    Some(found)
}

/// Case-insensitive match of `pattern` as typed, in one piece. `None` if `text` doesn't
/// contain it, else a score that is higher the nearer the start it is, and for a start
/// of a word. An empty pattern scores 0.
pub fn contains_score(pattern: &str, text: &str) -> Option<i64> {
    let at = contains_positions(pattern, text)?.first().copied();
    let Some(at) = at else { return Some(0) };
    let before = text.to_lowercase().chars().nth(at.wrapping_sub(1)).filter(|_| at > 0);
    let word_start = at == 0 || before.is_some_and(|c| matches!(c, '-' | '_' | '.' | '/' | ':' | ' '));
    Some(1000 - at as i64 * 2 + if word_start { 20 } else { 0 })
}

/// The character positions of `pattern`'s first occurrence in `text`, for highlighting;
/// `None` when it isn't there, empty for an empty pattern.
pub fn contains_positions(pattern: &str, text: &str) -> Option<Vec<usize>> {
    if pattern.is_empty() {
        return Some(Vec::new());
    }
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let at = text.windows(pattern.len()).position(|window| window == pattern.as_slice())?;
    Some((at..at + pattern.len()).collect())
}

/// The single best-scoring candidate, or `None` if nothing matches.
pub fn best_match<'a>(pattern: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    candidates.filter_map(|c| score(pattern, c).map(|s| (s, c))).max_by_key(|(s, _)| *s).map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_needs_the_text_in_one_piece() {
        assert!(contains_score("traefik", "kube-system traefik-9bcdbbd9").is_some());
        assert_eq!(contains_score("tfk", "traefik"), None);
        assert_eq!(contains_positions("ABC", "xabcx"), Some(vec![1, 2, 3]));
        assert!(contains_score("web", "web-1").unwrap() > contains_score("web", "my-app-web").unwrap());
        assert_eq!(contains_score("", "anything"), Some(0));
    }

    #[test]
    fn empty_pattern_matches_everything() {
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn positions_follow_the_same_greedy_walk_as_score() {
        assert_eq!(positions("ns", "namespaces"), Some(vec![0, 4]));
        assert_eq!(positions("", "abc"), Some(vec![]));
        assert_eq!(positions("ba", "ab"), None);
        assert_eq!(positions("POD", "my-pod"), Some(vec![3, 4, 5]));
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
