//! Sequential B=1 form of the paper's reverse-position mask layout.
//!
//! Bit d*q+b constrains a candidate ending d bytes in the future in bucket b.
//! Masks are zero in padding (d >= the bucket's common suffix length).
//! Before byte 0, the first length-1 bits of each bucket are one, preventing
//! candidates whose suffix would begin before the input. For each byte:
//! R |= mask[byte]; report zero bits in row 0; R >>= q (zero fill).
use crate::{
    Match, Patterns,
    filter::{Candidate, SearchStats},
    grouping::{self, Coefficients, Strategy},
};
use std::num::NonZeroUsize;

#[derive(Debug)]
pub struct PaperFilter {
    patterns: Patterns,
    buckets: Vec<Vec<usize>>,
    effective_lengths: Vec<usize>,
    masks: Vec<Vec<u64>>,
    initial: Vec<u64>,
    width: usize,
}
impl PaperFilter {
    pub fn new(
        patterns: Patterns,
        bucket_count: NonZeroUsize,
        filter_length: NonZeroUsize,
        strategy: Strategy,
        coefficients: Coefficients,
    ) -> Self {
        let buckets = grouping::partition(&patterns, bucket_count, strategy, coefficients);
        let effective_lengths: Vec<_> = buckets
            .iter()
            .map(|bucket| {
                bucket
                    .iter()
                    .map(|&id| patterns.as_slice()[id].len())
                    .min()
                    .unwrap()
                    .min(filter_length.get())
            })
            .collect();
        let rows = effective_lengths.iter().copied().max().unwrap_or(0);
        let q = buckets.len();
        let width = q * rows;
        let mut masks = vec![vec![0; width.div_ceil(64)]; 256];
        let mut initial = vec![0; width.div_ceil(64)];
        for (b, bucket) in buckets.iter().enumerate() {
            let length = effective_lengths[b];
            for d in 0..length {
                let bit = d * q + b;
                let flag = 1u64 << (bit % 64);
                for mask in &mut masks {
                    mask[bit / 64] |= flag;
                }
                for &id in bucket {
                    let pattern = &patterns.as_slice()[id];
                    masks[pattern[pattern.len() - 1 - d] as usize][bit / 64] &= !flag;
                }
                if d + 1 < length {
                    initial[bit / 64] |= flag;
                }
            }
        }
        Self {
            patterns,
            buckets,
            effective_lengths,
            masks,
            initial,
            width,
        }
    }
    pub fn buckets(&self) -> &[Vec<usize>] {
        &self.buckets
    }
    pub fn effective_lengths(&self) -> &[usize] {
        &self.effective_lengths
    }
    pub fn state_width(&self) -> usize {
        self.width
    }
    pub fn word_count(&self) -> usize {
        self.initial.len()
    }
    /// Mask payload only; excludes Vec headers, allocator overhead and patterns.
    pub fn mask_table_bytes(&self) -> usize {
        256 * self.word_count() * 8
    }
    pub fn initial_state(&self) -> &[u64] {
        &self.initial
    }
    pub fn mask(&self, byte: u8) -> &[u64] {
        &self.masks[byte as usize]
    }

    pub fn scan_candidates(&self, input: &[u8], mut report: impl FnMut(Candidate)) {
        let q = self.buckets.len();
        if q == 0 {
            return;
        }
        let mut state = self.initial.clone();
        for (index, &byte) in input.iter().enumerate() {
            for (word, &mask) in state.iter_mut().zip(&self.masks[byte as usize]) {
                *word |= mask;
            }
            for b in 0..q {
                if state[b / 64] & (1u64 << (b % 64)) == 0 {
                    report(Candidate {
                        bucket_id: b,
                        end: index + 1,
                    });
                }
            }
            // Ascending writes preserve source words during the right shift.
            let words = q / 64;
            let bits = q % 64;
            for destination in 0..state.len() {
                let source = destination + words;
                let mut value = state.get(source).copied().unwrap_or(0) >> bits;
                if bits != 0 {
                    value |= state.get(source + 1).copied().unwrap_or(0) << (64 - bits);
                }
                state[destination] = value;
            }
        }
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
    /// Reports by end position, then bucket and member order (not original ID).
    pub fn scan(&self, input: &[u8], report: impl FnMut(Match)) {
        self.search::<false>(input, report);
    }
    pub fn find_all(&self, input: &[u8]) -> Vec<Match> {
        let mut matches = Vec::new();
        self.scan(input, |m| matches.push(m));
        matches
    }
    pub fn statistics(&self, input: &[u8]) -> SearchStats {
        self.search::<true>(input, |_| {})
    }
}
