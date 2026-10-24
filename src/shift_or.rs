//! Single-pattern Shift-OR with the article's explicit start state.
//!
//! Bit 0 is always active (zero); bit j represents a matching prefix of length
//! j. A u64 therefore holds patterns of 1..=63 bytes, including the start bit.

use thiserror::Error;

use crate::Match;

pub const MAX_PATTERN_LEN: usize = u64::BITS as usize - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("pattern length {length} is outside 1..={MAX_PATTERN_LEN}")]
pub struct InvalidPatternLength {
    pub length: usize,
}

/// A single pattern, whose matches have pattern ID 0.
#[derive(Debug)]
pub struct ShiftOr {
    masks: [u64; 256],
    length: usize,
    state_mask: u64,
}

/// State immediately after consuming the byte at `end - 1`.
/// Only the low `pattern length + 1` bits are used; zero means active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceStep {
    pub end: usize,
    pub byte: u8,
    pub mask: u64,
    pub state: u64,
    pub matched: bool,
}

impl ShiftOr {
    pub fn new(pattern: &[u8]) -> Result<Self, InvalidPatternLength> {
        let classes: Vec<Vec<u8>> = pattern.iter().map(|&byte| vec![byte]).collect();
        Self::from_classes(&classes)
    }

    /// Each entry lists the allowed bytes at one position. Duplicate bytes are
    /// harmless; an empty class accepts no byte. No textual regex syntax is parsed.
    pub fn from_classes(classes: &[Vec<u8>]) -> Result<Self, InvalidPatternLength> {
        let length = classes.len();
        if !(1..=MAX_PATTERN_LEN).contains(&length) {
            return Err(InvalidPatternLength { length });
        }
        // This also works at length 63, without shifting by 64.
        let state_mask = u64::MAX >> (MAX_PATTERN_LEN - length);
        let mut masks = [state_mask & !1; 256];
        for (position, class) in classes.iter().enumerate() {
            for &byte in class {
                masks[usize::from(byte)] &= !(1 << (position + 1));
            }
        }
        Ok(Self {
            masks,
            length,
            state_mask,
        })
    }

    pub fn state_width(&self) -> usize {
        self.length + 1
    }

    pub fn initial_state(&self) -> u64 {
        self.state_mask & !1
    }

    pub fn mask(&self, byte: u8) -> u64 {
        self.masks[usize::from(byte)]
    }

    fn walk(&self, input: &[u8], mut observe: impl FnMut(TraceStep)) {
        let mut state = self.initial_state();
        for (offset, &byte) in input.iter().enumerate() {
            let mask = self.mask(byte);
            state = ((state << 1) | mask) & self.state_mask;
            observe(TraceStep {
                end: offset + 1,
                byte,
                mask,
                state,
                matched: state & (1 << self.length) == 0,
            });
        }
    }

    /// Reports all matches, including overlaps, in start-offset order.
    /// Each call starts from the initial state.
    pub fn scan(&self, input: &[u8], mut report: impl FnMut(Match)) {
        self.walk(input, |step| {
            if step.matched {
                report(Match {
                    pattern_id: 0,
                    start: step.end - self.length,
                    end: step.end,
                });
            }
        });
    }

    pub fn find_all(&self, input: &[u8]) -> Vec<Match> {
        let mut matches = Vec::new();
        self.scan(input, |m| matches.push(m));
        matches
    }

    /// Records the mask and resulting state for every input byte.
    pub fn trace(&self, input: &[u8]) -> Vec<TraceStep> {
        let mut steps = Vec::new();
        self.walk(input, |step| steps.push(step));
        steps
    }
}
