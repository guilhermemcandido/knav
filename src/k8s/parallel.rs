//! Spreading a pure per-item computation over the cores, for lists of tens of thousands.

/// Below this many items the threads cost more than they save.
const MIN_PARALLEL: usize = 4000;

/// `items.iter().map(f).collect()`, split across up to 8 threads, order kept.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    if items.len() < MIN_PARALLEL || threads == 1 {
        return items.iter().map(f).collect();
    }
    let chunk = items.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let workers: Vec<_> = items.chunks(chunk).map(|part| scope.spawn(|| part.iter().map(&f).collect::<Vec<R>>())).collect();
        workers.into_iter().flat_map(|w| w.join().expect("row worker panicked")).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_order_and_matches_a_plain_map() {
        let items: Vec<u32> = (0..10_000).collect();
        assert_eq!(par_map(&items, |n| n * 2), items.iter().map(|n| n * 2).collect::<Vec<_>>());
        assert_eq!(par_map(&items[..10], |n| n + 1), (1..=10).collect::<Vec<u32>>());
    }
}
