//! Exhaustive reference matcher. No filtering or skipping is performed.

use crate::{Match, Patterns};

#[derive(Debug)]
pub struct Naive {
    patterns: Patterns,
}

impl Naive {
    pub fn new(patterns: Patterns) -> Self {
        Self { patterns }
    }

    /// Reports every occurrence, ordered by pattern ID and then start offset.
    /// Callbacks let later benchmarks consume matches without storing them all.
    pub fn scan(&self, input: &[u8], mut report: impl FnMut(Match)) {
        for (pattern_id, pattern) in self.patterns.as_slice().iter().enumerate() {
            for (start, window) in input.windows(pattern.len()).enumerate() {
                if window == pattern {
                    report(Match {
                        pattern_id,
                        start,
                        end: start + pattern.len(),
                    });
                }
            }
        }
    }

    pub fn find_all(&self, input: &[u8]) -> Vec<Match> {
        let mut matches = Vec::new();
        self.scan(input, |m| matches.push(m));
        matches
    }
}
