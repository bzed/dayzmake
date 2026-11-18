//! The sort of the MSVC C runtime (`qsort`), which DayZ's generator uses wherever upstream Recast and
//! Detour call `qsort`: the BV tree of Detour (`subdivide`) and the hole and diagonal sorts of the
//! contour builder (`mergeRegionHoles`). The order of equal keys is observable in the output.

use std::cmp::Ordering;

/// The comparison of two items, `Equal` for an index outside the slice.
fn cmp_at<T>(items: &[T], a: usize, b: usize, cmp: &impl Fn(&T, &T) -> Ordering) -> Ordering {
    match (items.get(a), items.get(b)) {
        (Some(x), Some(y)) => cmp(x, y),
        _ => Ordering::Equal,
    }
}

/// The largest range that the short sort handles.
const QSORT_CUTOFF: usize = 8;

/// An in-place sort with the observable element order of the MSVC C runtime's `qsort`: a selection
/// sort for ranges of up to 8 items, else a median-of-three quicksort that ranges are split by.
/// It is not stable, and with a comparator that ties, the order of equal items is the one that the
/// shipped DayZ meshes have (spec 14 item 3 and ruling R18). The ranges are kept on an explicit stack.
pub(crate) fn msvc_qsort<T>(items: &mut [T], cmp: impl Fn(&T, &T) -> Ordering) {
    use Ordering::{Equal, Greater};
    let Some(last) = items.len().checked_sub(1) else {
        return;
    };
    let mut stack = vec![(0usize, last)];
    while let Some((lo, mut hi)) = stack.pop() {
        if hi <= lo {
            continue;
        }
        let size = hi - lo + 1;
        if size <= QSORT_CUTOFF {
            while hi > lo {
                let mut max = lo;
                for p in lo + 1..=hi {
                    if cmp_at(items, p, max, &cmp) == Greater {
                        max = p;
                    }
                }
                items.swap(max, hi);
                hi -= 1;
            }
            continue;
        }
        let mut mid = lo + size / 2;
        if cmp_at(items, lo, mid, &cmp) == Greater {
            items.swap(lo, mid);
        }
        if cmp_at(items, lo, hi, &cmp) == Greater {
            items.swap(lo, hi);
        }
        if cmp_at(items, mid, hi, &cmp) == Greater {
            items.swap(mid, hi);
        }
        let mut loguy = lo;
        let mut higuy = hi;
        loop {
            if mid > loguy {
                loop {
                    loguy += 1;
                    if !(loguy < mid && cmp_at(items, loguy, mid, &cmp) != Greater) {
                        break;
                    }
                }
            }
            if mid <= loguy {
                loop {
                    loguy += 1;
                    if !(loguy <= hi && cmp_at(items, loguy, mid, &cmp) != Greater) {
                        break;
                    }
                }
            }
            loop {
                higuy = higuy.saturating_sub(1);
                if !(higuy > mid && cmp_at(items, higuy, mid, &cmp) == Greater) {
                    break;
                }
            }
            if higuy < loguy {
                break;
            }
            items.swap(loguy, higuy);
            if mid == higuy {
                mid = loguy;
            }
        }
        higuy += 1;
        if mid < higuy {
            loop {
                higuy = higuy.saturating_sub(1);
                if !(higuy > mid && cmp_at(items, higuy, mid, &cmp) == Equal) {
                    break;
                }
            }
        }
        // Not `else`: when the loop above ran down to `mid`, the items below the pivot that equal it
        // are skipped too, which changes the order for equal keys. The shipped meshes confirm it.
        if mid >= higuy {
            loop {
                higuy = higuy.saturating_sub(1);
                if !(higuy > lo && cmp_at(items, higuy, mid, &cmp) == Equal) {
                    break;
                }
            }
        }
        // The ranges are disjoint, so the order they are sorted in does not change the result. The
        // smaller one is popped first, which keeps the stack at O(log n) entries.
        let left = (lo < higuy).then_some((lo, higuy));
        let right = (loguy < hi).then_some((loguy, hi));
        let len = |r: &(usize, usize)| r.1 - r.0;
        let (first, second) = match (left, right) {
            (Some(l), Some(r)) if len(&l) < len(&r) => (Some(r), Some(l)),
            other => other,
        };
        stack.extend(first);
        stack.extend(second);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Items with the given keys; the second value is the position before the sort.
    fn items(keys: &[u16]) -> Vec<(u16, i32)> {
        keys.iter()
            .enumerate()
            .map(|(i, &k)| (k, i32::try_from(i).expect("small")))
            .collect()
    }

    /// Sorts the keys and returns the original positions in the sorted order.
    fn order(keys: &[u16]) -> Vec<i32> {
        let mut v = items(keys);
        msvc_qsort(&mut v, |a, b| a.0.cmp(&b.0));
        let sorted: Vec<u16> = v.iter().map(|x| x.0).collect();
        assert!(sorted.is_sorted(), "not sorted: {sorted:?}");
        v.iter().map(|x| x.1).collect()
    }

    #[test]
    fn msvc_qsort_matches_reference_orders() {
        // One and two items. The short sort swaps the last with the largest, and the first of
        // equal items is the largest, so equal items move.
        assert_eq!(order(&[]), Vec::<i32>::new());
        assert_eq!(order(&[5]), vec![0]);
        assert_eq!(order(&[1, 1]), vec![1, 0]);
        assert_eq!(order(&[1, 2]), vec![0, 1]);
        assert_eq!(order(&[2, 1]), vec![1, 0]);
        // Three equal items: swap(0, 2) gives 2,1,0, then swap(0, 1) gives 1,2,0.
        assert_eq!(order(&[1, 1, 1]), vec![1, 2, 0]);
        // Eight equal items, the whole range is one short sort.
        assert_eq!(order(&[3; 8]), vec![1, 2, 3, 4, 5, 6, 7, 0]);
        // Eight items with two keys, derived by hand step by step.
        assert_eq!(
            order(&[2, 1, 2, 1, 2, 1, 2, 1]),
            vec![1, 5, 3, 7, 4, 6, 2, 0]
        );
        // Nine equal items: the partition leaves them alone, and no range is left to sort.
        assert_eq!(order(&[4; 9]), (0..9).collect::<Vec<i32>>());
        // Nine descending keys: the median of three and three swaps sort them.
        assert_eq!(
            order(&[8, 7, 6, 5, 4, 3, 2, 1, 0]),
            vec![8, 7, 6, 5, 4, 3, 2, 1, 0]
        );
        // Twelve items, keys 1,0,1,0,..., by hand (ids are the positions before the sort):
        // - lo=0, hi=11, mid=6 (key 1). cmp(lo, hi) is Greater: swap, so 11 is first and 0 last.
        //   Then loguy runs to 12 and higuy stops at 10, so the loop ends at once. The tail
        //   skips the equal 10, stops at 9: range [0, 9] is left, and 10, 0 stay.
        // - [0, 9]: mid=5 (id 5, key 0). swap(2, 7) and swap(4, 5); the second one has
        //   mid == higuy, so the pivot follows to index 4. loguy=5, higuy=4 end the loop, the tail
        //   gives higuy=5, then 4, and the second check runs, since mid >= higuy: it skips the
        //   equal ids 3, 7, 1 and stops at lo. The ranges are [] and [5, 9], which the short sort
        //   orders as 9, 2, 8, 6, 4.
        assert_eq!(
            order(&[1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0]),
            vec![11, 1, 7, 3, 5, 9, 2, 8, 6, 4, 10, 0]
        );
        // Thirteen items; the expected order is from an independent port of the same description.
        assert_eq!(
            order(&[0, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0]),
            vec![0, 8, 12, 4, 3, 5, 2, 7, 1, 9, 10, 11, 6]
        );
    }

    #[test]
    fn msvc_qsort_all_equal_terminates() {
        let mut v = items(&[7; 1000]);
        msvc_qsort(&mut v, |_, _| Ordering::Equal);
        let mut ids: Vec<i32> = v.iter().map(|x| x.1).collect();
        ids.sort_unstable();
        assert_eq!(ids, (0..1000).collect::<Vec<i32>>());
    }

    #[test]
    fn msvc_qsort_sorts_larger_inputs() {
        // A fixed pseudo-random sequence with many ties, and sorted and reversed ones.
        let mut x = 12345u32;
        let random: Vec<u16> = (0..500)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                u16::try_from((x >> 16) % 37).expect("small")
            })
            .collect();
        let sorted: Vec<u16> = (0..300).collect();
        let reversed: Vec<u16> = (0..300).rev().collect();
        for keys in [random, sorted, reversed] {
            let got = order(&keys);
            let mut seen = got.clone();
            seen.sort_unstable();
            let n = i32::try_from(keys.len()).expect("small");
            assert_eq!(seen, (0..n).collect::<Vec<i32>>());
        }
    }
}
