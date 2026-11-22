//! CSS coverage ranges: Playwright's `convertToDisjointRanges`.

/// A used / unused range from `CSS.stopRuleUsageTracking`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountedRange {
    pub start: u64,
    pub end: u64,
    pub count: u64,
}

/// A covered range of text (`{ start, end }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Range {
    pub start: u64,
    pub end: u64,
}

/// Flattens nested counted ranges into the covered, disjoint ranges.
pub fn disjoint_ranges(nested: &[CountedRange]) -> Vec<Range> {
    // (offset, is_end, range)
    let mut points: Vec<(u64, bool, &CountedRange)> = nested
        .iter()
        .flat_map(|r| [(r.start, false, r), (r.end, true, r)])
        .collect();
    points.sort_by(|a, b| {
        a.0.cmp(&b.0)
            // Ends before starts at the same offset.
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| {
                let (la, lb) = (a.2.end - a.2.start, b.2.end - b.2.start);
                if a.1 { la.cmp(&lb) } else { lb.cmp(&la) }
            })
    });
    let mut stack: Vec<u64> = Vec::new();
    let mut results: Vec<Range> = Vec::new();
    let mut last = 0;
    for (offset, is_end, range) in points {
        if stack.last().is_some_and(|c| *c > 0) && last < offset {
            match results.last_mut() {
                Some(r) if r.end == last => r.end = offset,
                _ => results.push(Range {
                    start: last,
                    end: offset,
                }),
            }
        }
        last = offset;
        if is_end {
            stack.pop();
        } else {
            stack.push(range.count);
        }
    }
    results.retain(|r| r.end - r.start > 1);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: u64, end: u64, count: u64) -> CountedRange {
        CountedRange { start, end, count }
    }

    #[test]
    fn flattens_like_playwright() {
        assert_eq!(
            disjoint_ranges(&[r(0, 21, 1), r(22, 47, 0)]),
            [Range { start: 0, end: 21 }]
        );
        // Adjacent used ranges merge.
        assert_eq!(
            disjoint_ranges(&[r(0, 10, 1), r(10, 20, 1)]),
            [Range { start: 0, end: 20 }]
        );
        // An unused range nested in a used one splits it.
        assert_eq!(
            disjoint_ranges(&[r(0, 30, 1), r(10, 20, 0)]),
            [Range { start: 0, end: 10 }, Range { start: 20, end: 30 }]
        );
        // Ranges of one character are dropped.
        assert_eq!(disjoint_ranges(&[r(5, 6, 1)]), []);
        assert_eq!(disjoint_ranges(&[]), []);
    }
}
