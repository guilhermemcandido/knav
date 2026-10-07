//! Home's favorites live in `config.toml` (`home.favorites`); these decide what Home
//! shows and how a pin changes the list.

use crate::config::HomeConfig;

/// The favorites Home shows in place of the categories, or `None` for every kind.
/// `session_all` is what `f` chose this session, over the configured default view.
pub(crate) fn home(config: &HomeConfig, session_all: Option<bool>) -> Option<&[String]> {
    let configured = match config.default_view.as_str() {
        "favorites" => false,
        "all" => true,
        _ => config.favorites.is_empty(),
    };
    let all = session_all.unwrap_or(configured);
    (!all).then_some(config.favorites.as_slice())
}

/// `labels` with `label` added, or removed if it was there; and whether it is pinned now.
pub(crate) fn toggled(labels: &[String], label: &str) -> (Vec<String>, bool) {
    if labels.iter().any(|l| l == label) {
        (labels.iter().filter(|l| *l != label).cloned().collect(), false)
    } else {
        (labels.iter().cloned().chain([label.to_string()]).collect(), true)
    }
}

/// `labels` with `label` one place earlier (`back`) or later; `None` at the ends.
pub(crate) fn nudged(labels: &[String], label: &str, back: bool) -> Option<Vec<String>> {
    let at = labels.iter().position(|l| l == label)?;
    let to = if back { at.checked_sub(1)? } else { Some(at + 1).filter(|i| *i < labels.len())? };
    let mut out = labels.to_vec();
    out.swap(at, to);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_default_view_is_favorites_once_there_are_some() {
        let mut config = HomeConfig::default();
        assert_eq!(home(&config, None), None);
        config.favorites = labels(&["Pods"]);
        assert_eq!(home(&config, None), Some(&labels(&["Pods"])[..]));
        config.default_view = "all".into();
        assert_eq!(home(&config, None), None);
        assert_eq!(home(&config, Some(false)), Some(&labels(&["Pods"])[..]));
        config.default_view = "favorites".into();
        config.favorites.clear();
        assert_eq!(home(&config, None), Some(&[][..]));
    }

    #[test]
    fn toggling_adds_then_removes() {
        let (once, pinned) = toggled(&labels(&["A"]), "B");
        assert_eq!((once.clone(), pinned), (labels(&["A", "B"]), true));
        assert_eq!(toggled(&once, "A"), (labels(&["B"]), false));
    }

    #[test]
    fn nudging_swaps_neighbours_and_stops_at_the_ends() {
        let all = labels(&["A", "B", "C"]);
        assert_eq!(nudged(&all, "A", true), None);
        assert_eq!(nudged(&all, "A", false), Some(labels(&["B", "A", "C"])));
        assert_eq!(nudged(&all, "C", false), None);
    }
}
