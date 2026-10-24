//! Integer and SIMD sequential/batched forms of the paper's reverse-position mask layout.
//!
//! Bit d*q+b constrains a candidate ending d bytes in the future in bucket b.
//! Masks are zero in padding (d >= the bucket's common suffix length).
//! Before byte 0, the first length-1 bits of each bucket are one, preventing
//! candidates whose suffix would begin before the input. For each byte:
//! R |= mask[byte]; report zero bits in row 0; R >>= q (zero fill).
//! Batching ORs B shifted masks before reporting B rows and shifting by B*q.
use crate::simd::{self, Backend};
use crate::{
    Match, Patterns,
    filter::{Candidate, SearchStats, verify},
    grouping::{self, Coefficients, Strategy},
};
use clap::ValueEnum;
use hashbrown::HashMap;
use serde::Serialize;
use std::num::NonZeroUsize;

/// Verification strategy; filtering and candidate order are unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Verification {
    Exhaustive,
    Hash,
}

/// Number of input bytes combined before shifting the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BatchSize {
    #[value(name = "1")]
    One = 1,
    #[value(name = "2")]
    Two = 2,
    #[value(name = "4")]
    Four = 4,
    #[value(name = "8")]
    Eight = 8,
}
impl BatchSize {
    pub fn get(self) -> usize {
        self as usize
    }
}

/// Low bits of the next byte included in each mask lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SuperCharacterBits {
    #[value(name = "0")]
    Zero = 0,
    #[value(name = "4")]
    Four = 4,
    #[value(name = "8")]
    Eight = 8,
}
impl SuperCharacterBits {
    pub fn get(self) -> usize {
        self as usize
    }
}

#[derive(Debug)]
pub struct FdrFilter {
    patterns: Patterns,
    buckets: Vec<Vec<usize>>,
    effective_lengths: Vec<usize>,
    masks: Vec<Vec<u64>>,
    super_masks: Vec<Vec<u64>>,
    super_character_bits: SuperCharacterBits,
    initial: Vec<u64>,
    width: usize,
    batch_size: BatchSize,
    backend: Backend,
    verification: Verification,
    verification_tables: Vec<HashMap<Vec<u8>, Vec<usize>>>,
}

impl FdrFilter {
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
        let q = buckets.len();
        let width = q * effective_lengths.iter().copied().max().unwrap_or(0);
        let mut initial = vec![0; width.div_ceil(64)];
        for (b, &length) in effective_lengths.iter().enumerate() {
            for d in 0..length.saturating_sub(1) {
                let bit = d * q + b;
                initial[bit / 64] |= 1 << (bit % 64);
            }
        }
        let mut filter = Self {
            patterns,
            buckets,
            effective_lengths,
            masks: Vec::new(),
            super_masks: Vec::new(),
            super_character_bits: SuperCharacterBits::Zero,
            initial,
            width,
            batch_size: BatchSize::One,
            backend: Backend::Integer,
            verification: Verification::Exhaustive,
            verification_tables: Vec::new(),
        };
        filter.masks = filter.build_masks(0);
        filter
    }

    /// Masks indexed by `(byte << bits) | (next & (2^bits - 1))`. With zero
    /// bits this is the byte-only table. The final pattern byte (d = 0)
    /// accepts every next-byte value.
    fn build_masks(&self, bits: usize) -> Vec<Vec<u64>> {
        let variants = 1 << bits;
        let q = self.buckets.len();
        let mut masks = vec![vec![0; self.initial.len()]; 256 * variants];
        for (b, bucket) in self.buckets.iter().enumerate() {
            for d in 0..self.effective_lengths[b] {
                let bit = d * q + b;
                let flag = 1u64 << (bit % 64);
                for mask in &mut masks {
                    mask[bit / 64] |= flag;
                }
                for &id in bucket {
                    let pattern = &self.patterns.as_slice()[id];
                    let position = pattern.len() - 1 - d;
                    let base = usize::from(pattern[position]) << bits;
                    let nexts = if d == 0 {
                        0..variants
                    } else {
                        let next = usize::from(pattern[position + 1]) & (variants - 1);
                        next..next + 1
                    };
                    for next in nexts {
                        masks[base | next][bit / 64] &= !flag;
                    }
                }
            }
        }
        masks
    }

    /// Reject unsupported architectures; layout and candidate extraction stay shared.
    pub fn with_backend(mut self, backend: Backend) -> Result<Self, &'static str> {
        if !backend.is_supported() {
            return Err("requested SIMD backend is unavailable on this architecture");
        }
        self.backend = backend;
        Ok(self)
    }
    /// Index each bucket's common suffix. Equal suffixes retain all pattern IDs
    /// and are always followed by comparison of the entire original pattern.
    pub fn with_verification(mut self, verification: Verification) -> Self {
        self.verification = verification;
        self.verification_tables.clear();
        if verification == Verification::Hash {
            for (b, bucket) in self.buckets.iter().enumerate() {
                let mut table: HashMap<Vec<u8>, Vec<usize>> = HashMap::new();
                for &id in bucket {
                    let pattern = &self.patterns.as_slice()[id];
                    let suffix = pattern[pattern.len() - self.effective_lengths[b]..].to_vec();
                    table.entry(suffix).or_default().push(id);
                }
                self.verification_tables.push(table);
            }
        }
        self
    }
    /// Add byte-pair masks; byte-only masks remain for the end of input.
    pub fn with_super_character(mut self, bits: SuperCharacterBits) -> Self {
        self.super_character_bits = bits;
        self.super_masks = match bits {
            SuperCharacterBits::Zero => Vec::new(),
            _ => self.build_masks(bits.get()),
        };
        self
    }
    /// Select batching without rebuilding the buckets or masks.
    pub fn with_batch_size(mut self, batch_size: BatchSize) -> Self {
        self.batch_size = batch_size;
        self
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }
    pub fn verification(&self) -> Verification {
        self.verification
    }
    pub fn super_character_bits(&self) -> SuperCharacterBits {
        self.super_character_bits
    }
    pub fn batch_size(&self) -> BatchSize {
        self.batch_size
    }
    pub fn buckets(&self) -> &[Vec<usize>] {
        &self.buckets
    }
    pub fn effective_lengths(&self) -> &[usize] {
        &self.effective_lengths
    }
    pub fn state_width(&self) -> usize {
        self.width + self.buckets.len() * (self.batch_size.get() - 1)
    }
    pub fn word_count(&self) -> usize {
        self.state_width().div_ceil(64)
    }
    /// Mask payload only; excludes Vec headers, allocator overhead and patterns.
    pub fn mask_table_bytes(&self) -> usize {
        (self.masks.len() + self.super_masks.len()) * self.initial.len() * 8
    }
    /// Logical payload: suffix bytes and usize IDs. Excludes HashMap capacity,
    /// control bytes, Vec headers/capacity, allocator overhead and patterns.
    pub fn verification_table_bytes(&self) -> usize {
        self.verification_tables
            .iter()
            .flat_map(|table| table.iter())
            .map(|(suffix, ids)| suffix.len() + ids.len() * size_of::<usize>())
            .sum()
    }
    /// Sequential initial state; batching extends it with zero padding.
    pub fn initial_state(&self) -> &[u64] {
        &self.initial
    }
    pub fn mask(&self, byte: u8) -> &[u64] {
        &self.masks[usize::from(byte)]
    }
    /// Without a next byte, use the byte-only mask, accepting any next byte.
    pub fn super_character_mask(&self, byte: u8, next: Option<u8>) -> &[u64] {
        match next {
            Some(next) if !self.super_masks.is_empty() => {
                let bits = self.super_character_bits.get();
                let index = (usize::from(byte) << bits) | (usize::from(next) & ((1 << bits) - 1));
                &self.super_masks[index]
            }
            _ => self.mask(byte),
        }
    }

    /// Reports candidates by end position, then bucket.
    pub fn scan_candidates(&self, input: &[u8], mut report: impl FnMut(Candidate)) {
        let q = self.buckets.len();
        if q == 0 {
            return;
        }
        let batch = self.batch_size.get();
        let mut state = self.initial.clone();
        state.resize(self.word_count(), 0);
        for (block, bytes) in input.chunks(batch).enumerate() {
            let block_start = block * batch;
            // A = R | M[c0] | (M[c1] << q) | ...
            for (offset, &byte) in bytes.iter().enumerate() {
                let next = input.get(block_start + offset + 1).copied();
                let mask = self.super_character_mask(byte, next);
                simd::or_shifted(&mut state, mask, offset * q, self.backend);
            }
            // Row j holds the candidates ending at byte j of this block.
            for offset in 0..bytes.len() {
                for b in 0..q {
                    let bit = offset * q + b;
                    if state[bit / 64] & (1 << (bit % 64)) == 0 {
                        report(Candidate {
                            bucket_id: b,
                            end: block_start + offset + 1,
                        });
                    }
                }
            }
            shift_right(&mut state, bytes.len() * q);
        }
    }

    fn search<const STATS: bool>(
        &self,
        input: &[u8],
        mut report: impl FnMut(Match),
    ) -> SearchStats {
        let mut stats = SearchStats::default();
        self.scan_candidates(input, |Candidate { bucket_id, end }| {
            let ids = match self.verification {
                Verification::Exhaustive => &self.buckets[bucket_id][..],
                Verification::Hash => {
                    if STATS {
                        stats.hash_lookups += 1;
                    }
                    // The initial state guarantees end >= the common suffix length.
                    let length = self.effective_lengths[bucket_id];
                    self.verification_tables[bucket_id]
                        .get(&input[end - length..end])
                        .map_or(&[][..], Vec::as_slice)
                }
            };
            verify::<STATS>(&self.patterns, ids, input, end, &mut stats, &mut report);
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
    /// Separate pass so counter updates do not enter timed searches.
    pub fn statistics(&self, input: &[u8]) -> SearchStats {
        self.search::<true>(input, |_| {})
    }
}

/// Logical right shift with zero fill. Ascending writes preserve source words.
fn shift_right(state: &mut [u64], shift: usize) {
    let words = shift / 64;
    let bits = shift % 64;
    for destination in 0..state.len() {
        let source = destination + words;
        let mut value = state.get(source).copied().unwrap_or(0) >> bits;
        if bits != 0 {
            value |= state.get(source + 1).copied().unwrap_or(0) << (64 - bits);
        }
        state[destination] = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_suffix_keeps_duplicate_ids_and_requires_full_comparison() {
        let patterns =
            Patterns::new(vec![b"xab".to_vec(), b"yab".to_vec(), b"xab".to_vec()]).unwrap();
        let filter = FdrFilter::new(
            patterns,
            NonZeroUsize::new(1).unwrap(),
            NonZeroUsize::new(2).unwrap(),
            Strategy::Balanced,
            Coefficients::default(),
        )
        .with_verification(Verification::Hash);
        assert_eq!(
            filter.verification_tables[0].get(b"ab".as_slice()).unwrap(),
            &[0, 1, 2]
        );
        let found = filter.find_all(b"xabzab");
        assert_eq!(
            found.iter().map(|m| m.pattern_id).collect::<Vec<_>>(),
            vec![0, 2]
        );
        let stats = filter.statistics(b"xabzab");
        assert_eq!(stats.candidate_count, 2);
        assert_eq!(stats.hash_lookups, 2);
        assert_eq!(stats.full_comparisons, 6);
        assert_eq!(stats.false_positive_candidates, 1);
    }
}
