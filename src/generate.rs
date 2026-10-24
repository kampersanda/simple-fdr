//! Deterministic generated workloads, using SplitMix64 and rejection sampling.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Match;

/// A pattern to copy into the generated input at a known offset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Insertion {
    pub pattern_id: usize,
    pub start: usize,
}

/// Version 1 generates patterns first, input second, then applies insertions.
/// Lengths are sampled uniformly from the inclusive range. Bytes are sampled
/// uniformly from alphabet entries (repeated entries therefore act as weights).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub seed: u64,
    pub input_len: usize,
    pub pattern_count: usize,
    pub min_pattern_len: usize,
    pub max_pattern_len: usize,
    #[serde(default = "all_bytes")]
    pub alphabet: Vec<u8>,
    #[serde(default)]
    pub insertions: Vec<Insertion>,
}

fn all_bytes() -> Vec<u8> {
    (0..=255).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dataset {
    pub generator_version: u32,
    pub config: Config,
    pub patterns: Vec<Vec<u8>>,
    pub input: Vec<u8>,
    /// Guaranteed occurrences, not an exhaustive list of matches.
    pub planted_matches: Vec<Match>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateError(String);

impl fmt::Display for GenerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GenerateError {}

pub fn generate(config: &Config) -> Result<Dataset, GenerateError> {
    if config.min_pattern_len == 0 || config.max_pattern_len < config.min_pattern_len {
        return Err(GenerateError(
            "require 1 <= min_pattern_len <= max_pattern_len".into(),
        ));
    }
    if config.alphabet.is_empty() {
        return Err(GenerateError("alphabet must not be empty".into()));
    }

    let mut rng = SplitMix64(config.seed);
    let mut patterns = Vec::with_capacity(config.pattern_count);
    for _ in 0..config.pattern_count {
        let len =
            config.min_pattern_len + rng.below(config.max_pattern_len - config.min_pattern_len + 1);
        patterns.push(random_bytes(&mut rng, len, &config.alphabet));
    }

    // Validate all insertion ranges before copying, so every planted match survives.
    let mut planted_matches = Vec::with_capacity(config.insertions.len());
    for insertion in &config.insertions {
        let pattern = patterns
            .get(insertion.pattern_id)
            .ok_or_else(|| GenerateError(format!("unknown pattern ID {}", insertion.pattern_id)))?;
        let end = insertion
            .start
            .checked_add(pattern.len())
            .filter(|&end| end <= config.input_len)
            .ok_or_else(|| {
                GenerateError(format!(
                    "insertion for pattern {} exceeds input length",
                    insertion.pattern_id
                ))
            })?;
        planted_matches.push(Match {
            pattern_id: insertion.pattern_id,
            start: insertion.start,
            end,
        });
    }
    planted_matches.sort_by_key(|m| m.start);
    if planted_matches
        .windows(2)
        .any(|pair| pair[0].end > pair[1].start)
    {
        return Err(GenerateError("insertions must not overlap".into()));
    }

    let mut input = random_bytes(&mut rng, config.input_len, &config.alphabet);
    for m in &planted_matches {
        input[m.start..m.end].copy_from_slice(&patterns[m.pattern_id]);
    }
    Ok(Dataset {
        generator_version: 1,
        config: config.clone(),
        patterns,
        input,
        planted_matches,
    })
}

fn random_bytes(rng: &mut SplitMix64, len: usize, alphabet: &[u8]) -> Vec<u8> {
    (0..len)
        .map(|_| alphabet[rng.below(alphabet.len())])
        .collect()
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        let bound = bound as u64;
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next();
            if value >= threshold {
                return (value % bound) as usize;
            }
        }
    }
}
