//! Error types shared by pattern construction and workload generation.

use thiserror::Error;

use crate::shift_or::MAX_PATTERN_LEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("pattern {pattern_id} is empty")]
pub struct EmptyPattern {
    pub pattern_id: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct GenerateError(pub(crate) String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("pattern length {length} is outside 1..={MAX_PATTERN_LEN}")]
pub struct InvalidPatternLength {
    pub length: usize,
}
