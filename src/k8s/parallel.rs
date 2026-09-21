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

/// `items.sort_by(cmp)`, with the pieces sorted on separate threads and merged, for big lists.
pub fn par_sort_by<T: Send>(items: &mut Vec<T>, cmp: impl Fn(&T, &T) -> std::cmp::Ordering + Sync) {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    if items.len() < MIN_PARALLEL * 5 || threads == 1 {
        items.sort_by(&cmp);
        return;
    }
    let chunk = items.len().div_ceil(threads);
    let mut runs: Vec<Vec<T>> = Vec::new();
    while !items.is_empty() {
        let rest = items.split_off(chunk.min(items.len()));
        runs.push(std::mem::replace(items, rest));
    }
    std::thread::scope(|scope| {
        for run in runs.iter_mut() {
            scope.spawn(|| run.sort_by(&cmp));
        }
    });
    // Merge neighbouring runs until one is left.
    while runs.len() > 1 {
        let mut next = Vec::with_capacity(runs.len().div_ceil(2));
        let mut pending = runs.into_iter();
        while let Some(a) = pending.next() {
            match pending.next() {
                Some(b) => next.push(merge(a, b, &cmp)),
                None => next.push(a),
            }
        }
        runs = next;
    }
    *items = runs.pop().unwrap_or_default();
}

fn merge<T>(a: Vec<T>, b: Vec<T>, cmp: &impl Fn(&T, &T) -> std::cmp::Ordering) -> Vec<T> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut a, mut b) = (a.into_iter().peekable(), b.into_iter().peekable());
    loop {
        let take_a = match (a.peek(), b.peek()) {
            (Some(x), Some(y)) => cmp(x, y) != std::cmp::Ordering::Greater,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        out.push(if take_a { a.next() } else { b.next() }.expect("peeked"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parallel_sort_gives_the_same_order_as_a_plain_one() {
        let mut items: Vec<u64> = (0..60_000u64).map(|i| (i * 2_654_435_761) % 1_000_003).collect();
        let mut expected = items.clone();
        expected.sort_by(|a, b| a.cmp(b));
        par_sort_by(&mut items, |a, b| a.cmp(b));
        assert_eq!(items, expected);
    }

    #[test]
    fn keeps_the_order_and_matches_a_plain_map() {
        let items: Vec<u32> = (0..10_000).collect();
        assert_eq!(par_map(&items, |n| n * 2), items.iter().map(|n| n * 2).collect::<Vec<_>>());
        assert_eq!(par_map(&items[..10], |n| n + 1), (1..=10).collect::<Vec<u32>>());
    }
}
