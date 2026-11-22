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
    unimplemented!()
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
