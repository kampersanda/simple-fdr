//! Integer sequential and batched forms of the paper's reverse-position mask layout.
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
use hashbrown::HashMap;
use serde::Serialize;
use std::num::NonZeroUsize;

/// Verification strategy; filtering and candidate order are unchanged.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verification {
    #[default]
    Exhaustive,
    Hash,
}
impl std::str::FromStr for Verification {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "exhaustive" => Ok(Self::Exhaustive),
            "hash" => Ok(Self::Hash),
            _ => Err("verification must be exhaustive or hash"),
        }
    }
}
/// Number of input bytes combined before shifting the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchSize {
    One = 1,
    Two = 2,
    Four = 4,
    Eight = 8,
}
impl BatchSize {
    pub fn get(self) -> usize {
        self as usize
    }
}
impl std::str::FromStr for BatchSize {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "1" => Ok(Self::One),
            "2" => Ok(Self::Two),
            "4" => Ok(Self::Four),
            "8" => Ok(Self::Eight),
            _ => Err("batch size must be 1, 2, 4 or 8"),
        }
    }
}

/// Low bits of the next byte included in each mask lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuperCharacterBits {
    Zero = 0,
    Four = 4,
    Eight = 8,
}
impl SuperCharacterBits {
    pub fn get(self) -> usize {
        self as usize
    }
}
impl std::str::FromStr for SuperCharacterBits {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "0" => Ok(Self::Zero),
            "4" => Ok(Self::Four),
            "8" => Ok(Self::Eight),
            _ => Err("super character bits must be 0, 4 or 8"),
        }
    }
}

#[derive(Debug)]
pub struct FdrFilter {
    verification: Verification,
    verification_tables: Vec<HashMap<Vec<u8>, Vec<usize>>>,
    patterns: Patterns,
    buckets: Vec<Vec<usize>>,
    effective_lengths: Vec<usize>,
    masks: Vec<Vec<u64>>,
    super_masks: Vec<Vec<u64>>,
    super_character_bits: SuperCharacterBits,
    initial: Vec<u64>,
    width: usize,
    batch_size: BatchSize,
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
            verification: Verification::Exhaustive,
            verification_tables: Vec::new(),
            patterns,
            buckets,
            effective_lengths,
            masks,
            super_masks: Vec::new(),
            super_character_bits: SuperCharacterBits::Zero,
            initial,
            width,
            batch_size: BatchSize::One,
        }
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
    pub fn verification(&self) -> Verification {
        self.verification
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
    /// Build correlated byte-pair masks, retaining byte masks for end of input.
    /// The final pattern byte accepts every possible next-byte value.
    pub fn with_super_character(mut self, bits: SuperCharacterBits) -> Self {
        self.super_character_bits = bits;
        self.super_masks.clear();
        if bits == SuperCharacterBits::Zero || self.buckets.is_empty() {
            return self;
        }
        let variants = 1usize << bits.get();
        self.super_masks = vec![vec![0; self.initial.len()]; 256 * variants];
        let q = self.buckets.len();
        for (b, bucket) in self.buckets.iter().enumerate() {
            for d in 0..self.effective_lengths[b] {
                let bit = d * q + b;
                let flag = 1u64 << (bit % 64);
                for mask in &mut self.super_masks {
                    mask[bit / 64] |= flag;
                }
                for &id in bucket {
                    let pattern = &self.patterns.as_slice()[id];
                    let position = pattern.len() - 1 - d;
                    let base = (pattern[position] as usize) << bits.get();
                    if d == 0 {
                        for next in 0..variants {
                            self.super_masks[base | next][bit / 64] &= !flag;
                        }
                    } else {
                        let next = pattern[position + 1] as usize & (variants - 1);
                        self.super_masks[base | next][bit / 64] &= !flag;
                    }
                }
            }
        }
        self
    }
    pub fn super_character_bits(&self) -> SuperCharacterBits {
        self.super_character_bits
    }
    /// Without a next byte, use the byte-only mask, accepting any next byte.
    pub fn super_character_mask(&self, byte: u8, next: Option<u8>) -> &[u64] {
        if self.super_masks.is_empty() {
            return self.mask(byte);
        }
        match next {
            Some(next) => {
                let bits = self.super_character_bits.get();
                let index = ((byte as usize) << bits) | (next as usize & ((1 << bits) - 1));
                &self.super_masks[index]
            }
            None => self.mask(byte),
        }
    }
    /// Select batching without rebuilding the buckets or byte masks.
    pub fn with_batch_size(mut self, batch_size: BatchSize) -> Self {
        self.batch_size = batch_size;
        self
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
    /// Sequential initial state; batching extends it with zero padding.
    pub fn initial_state(&self) -> &[u64] {
        &self.initial
    }
    pub fn mask(&self, byte: u8) -> &[u64] {
        &self.masks[byte as usize]
    }

    pub fn scan_candidates(&self, input: &[u8], mut report: impl FnMut(Candidate)) {
        if self.batch_size != BatchSize::One {
            self.scan_batched(input, report);
            return;
        }
        let q = self.buckets.len();
        if q == 0 {
            return;
        }
        let mut state = self.initial.clone();
        for (index, &byte) in input.iter().enumerate() {
            for (word, &mask) in state
                .iter_mut()
                .zip(self.super_character_mask(byte, input.get(index + 1).copied()))
            {
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
    fn scan_batched(&self, input: &[u8], mut report: impl FnMut(Candidate)) {
        let q = self.buckets.len();
        if q == 0 {
            return;
        }
        let mut state = self.initial.clone();
        state.resize(self.word_count(), 0);
        for (block, bytes) in input.chunks(self.batch_size.get()).enumerate() {
            // A = R | M[c0] | (M[c1] << q) | ... . Each word is
            // accumulated locally: there is no per-byte state shift dependency.
            for (destination, word) in state.iter_mut().enumerate() {
                let mut combined = *word;
                for (offset, &byte) in bytes.iter().enumerate() {
                    let shift = offset * q;
                    let words = shift / 64;
                    let bits = shift % 64;
                    if destination >= words {
                        let source = destination - words;
                        let index = block * self.batch_size.get() + offset;
                        let mask = self.super_character_mask(byte, input.get(index + 1).copied());
                        combined |= mask.get(source).copied().unwrap_or(0) << bits;
                        if bits != 0 && source > 0 {
                            combined |= mask.get(source - 1).copied().unwrap_or(0) >> (64 - bits);
                        }
                    }
                }
                *word = combined;
            }
            // Row j retains the candidate ending at byte j of this block.
            for offset in 0..bytes.len() {
                for b in 0..q {
                    let bit = offset * q + b;
                    if state[bit / 64] & (1u64 << (bit % 64)) == 0 {
                        report(Candidate {
                            bucket_id: b,
                            end: block * self.batch_size.get() + offset + 1,
                        });
                    }
                }
            }
            let shift = bytes.len() * q;
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
            let ids = match self.verification {
                Verification::Exhaustive => &self.buckets[candidate.bucket_id][..],
                Verification::Hash => {
                    let length = self.effective_lengths[candidate.bucket_id];
                    // Initial state guarantees end >= common suffix length.
                    if STATS {
                        stats.hash_lookups += 1;
                    }
                    self.verification_tables[candidate.bucket_id]
                        .get(&input[candidate.end - length..candidate.end])
                        .map_or(&[][..], Vec::as_slice)
                }
            };
            for &id in ids {
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
