//! Deterministic generated workloads, using ChaCha8 and rand's uniform sampling.

pub use crate::errors::GenerateError;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::Match;

/// A pattern to copy into the generated input at a known offset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Insertion {
    pub pattern_id: usize,
    pub start: usize,
}

/// Version 2 generates patterns first, input second, then applies insertions.
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

pub fn generate(config: &Config) -> Result<Dataset, GenerateError> {
    if config.min_pattern_len == 0 || config.max_pattern_len < config.min_pattern_len {
        return Err(GenerateError(
            "require 1 <= min_pattern_len <= max_pattern_len".into(),
        ));
    }
    if config.alphabet.is_empty() {
        return Err(GenerateError("alphabet must not be empty".into()));
    }

    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut patterns = Vec::with_capacity(config.pattern_count);
    for _ in 0..config.pattern_count {
        // Use u64 ranges so sampling does not depend on the platform's usize width.
        let len = rng.random_range(config.min_pattern_len as u64..=config.max_pattern_len as u64)
            as usize;
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
        generator_version: 2,
        config: config.clone(),
        patterns,
        input,
        planted_matches,
    })
}

fn random_bytes(rng: &mut ChaCha8Rng, len: usize, alphabet: &[u8]) -> Vec<u8> {
    (0..len)
        .map(|_| alphabet[rng.random_range(0..alphabet.len() as u64) as usize])
        .collect()
}
