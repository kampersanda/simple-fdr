//! Byte-oriented multiple-pattern matching experiments.
//!
//! Pattern IDs are their positions in [`Patterns`]. Offsets are zero-based and
//! match ranges are half-open: `input[start..end]`.

pub mod errors;
pub mod filter;
pub mod generate;
pub mod grouping;
pub mod multiple_shift_or;
pub mod naive;
pub mod paper_filter;
pub mod shift_or;

pub use errors::EmptyPattern;
use serde::{Deserialize, Serialize};

/// Patterns in ID order. Identical byte strings retain separate IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patterns(Vec<Vec<u8>>);

impl Patterns {
    pub fn new(patterns: Vec<Vec<u8>>) -> Result<Self, EmptyPattern> {
        if let Some(pattern_id) = patterns.iter().position(Vec::is_empty) {
            return Err(EmptyPattern { pattern_id });
        }
        Ok(Self(patterns))
    }

    pub fn as_slice(&self) -> &[Vec<u8>] {
        &self.0
    }
}

/// One occurrence, including its pattern ID and half-open byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Match {
    pub pattern_id: usize,
    pub start: usize,
    pub end: usize,
}
