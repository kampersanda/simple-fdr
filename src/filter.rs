//! Explanation-model Grouping and Truncation with exhaustive verification.
//!
//! Each bucket shares prefix-position character classes and accepts every
//! distinct filtered length. A candidate is one (bucket, end) event, even when
//! several accepting positions are active. Verification checks all bucket members.

use std::num::NonZeroUsize;

use serde::Serialize;

use crate::{Match, Patterns, multiple_shift_or::MultipleShiftOr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Candidate {
    pub bucket_id: usize,
    pub end: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SearchStats {
    pub candidate_count: usize,
    pub full_comparisons: usize,
    pub false_positive_candidates: usize,
}

#[derive(Debug)]
pub struct Filter {
    patterns: Patterns,
    buckets: Vec<Vec<usize>>,
    accepting_lengths: Vec<Vec<usize>>,
    engine: MultipleShiftOr,
}

impl Filter {
    /// Groups consecutive IDs into at most `bucket_count` balanced buckets.
    /// `None` keeps whole patterns; `Some(length)` filters each pattern's suffix
    /// independently. This is not the common-suffix-length model of stage 4.
    pub fn new(
        patterns: Patterns,
        bucket_count: NonZeroUsize,
        filter_length: Option<NonZeroUsize>,
    ) -> Self {
        let count = patterns.as_slice().len();
        let bucket_count = bucket_count.get().min(count);
        let mut buckets = vec![Vec::new(); bucket_count];
        // Earlier buckets receive one extra pattern when division is uneven.
        let mut next_id = 0;
        for (bucket_id, bucket) in buckets.iter_mut().enumerate() {
            let size = count / bucket_count + usize::from(bucket_id < count % bucket_count);
            bucket.extend(next_id..next_id + size);
            next_id += size;
        }
        let mut classes = Vec::new();
        let mut accepting_lengths = Vec::new();
        for bucket in &buckets {
            let mut positions: Vec<Vec<u8>> = Vec::new();
            let mut lengths = Vec::new();
            for &id in bucket {
                let pattern = &patterns.as_slice()[id];
                let length = filter_length.map_or(pattern.len(), |n| n.get().min(pattern.len()));
                positions.resize_with(positions.len().max(length), Vec::new);
                for (position, &byte) in pattern[pattern.len() - length..].iter().enumerate() {
                    positions[position].push(byte);
                }
                lengths.push(length);
            }
            lengths.sort_unstable();
            lengths.dedup();
            classes.push(positions);
            accepting_lengths.push(lengths);
        }
        let engine = MultipleShiftOr::from_classes(&classes);
        Self {
            patterns,
            buckets,
            accepting_lengths,
            engine,
        }
    }

    pub fn state_width(&self) -> usize {
        self.engine.state_width()
    }
    pub fn word_count(&self) -> usize {
        self.engine.word_count()
    }
    pub fn buckets(&self) -> &[Vec<usize>] {
        &self.buckets
    }
    pub fn accepting_lengths(&self) -> &[Vec<usize>] {
        &self.accepting_lengths
    }

    /// Enumerates candidates without verifying or allocating snapshots.
    pub fn scan_candidates(&self, input: &[u8], mut report: impl FnMut(Candidate)) {
        let count = self.buckets.len();
        self.engine.walk(input, |end, _, state| {
            for (bucket_id, lengths) in self.accepting_lengths.iter().enumerate() {
                if lengths.iter().any(|&length| {
                    let bit = length * count + bucket_id;
                    state[bit / 64] & (1u64 << (bit % 64)) == 0
                }) {
                    report(Candidate { bucket_id, end });
                }
            }
        });
    }

    fn search<const STATS: bool>(
        &self,
        input: &[u8],
        mut report: impl FnMut(Match),
    ) -> SearchStats {
        let mut stats = SearchStats::default();
        self.scan_candidates(input, |candidate| {
            let mut matched = false;
            if STATS {
                stats.candidate_count += 1;
            }
            for &id in &self.buckets[candidate.bucket_id] {
                let pattern = &self.patterns.as_slice()[id];
                if pattern.len() > candidate.end {
                    continue;
                }
                if STATS {
                    stats.full_comparisons += 1;
                }
                let start = candidate.end - pattern.len();
                if input[start..candidate.end] == pattern[..] {
                    matched = true;
                    report(Match {
                        pattern_id: id,
                        start,
                        end: candidate.end,
                    });
                }
            }
            if STATS && !matched {
                stats.false_positive_candidates += 1;
            }
        });
        stats
    }

    /// Reports matches by end offset then pattern ID. Each call resets state.
    pub fn scan(&self, input: &[u8], report: impl FnMut(Match)) {
        self.search::<false>(input, report);
    }

    pub fn find_all(&self, input: &[u8]) -> Vec<Match> {
        let mut matches = Vec::new();
        self.scan(input, |m| matches.push(m));
        matches
    }

    /// Separate pass so counter updates do not enter timed searches.
    /// Comparisons skipped because the pattern exceeds `end` are not counted.
    pub fn statistics(&self, input: &[u8]) -> SearchStats {
        self.search::<true>(input, |_| {})
    }
}
