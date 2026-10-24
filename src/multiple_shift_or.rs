//! Position-major multiple-pattern Shift-OR with explicit start states.
//!
//! Bit j*q+k represents prefix length j of pattern k. Short patterns have
//! inactive dummy states up to the longest length. Words are least significant
//! first; zero means active.

use crate::{Match, Patterns};

#[derive(Debug)]
pub struct MultipleShiftOr {
    masks: Vec<Vec<u64>>,
    initial: Vec<u64>,
    lengths: Vec<usize>,
    width: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceStep {
    pub end: usize,
    pub byte: u8,
    pub mask: Vec<u64>,
    pub state: Vec<u64>,
    pub matches: Vec<Match>,
}

impl MultipleShiftOr {
    pub fn new(patterns: Patterns) -> Self {
        let patterns = patterns.as_slice();
        let count = patterns.len();
        let longest = patterns.iter().map(Vec::len).max().unwrap_or(0);
        let width = count * (longest + 1);
        let mut initial = vec![u64::MAX; width.div_ceil(64)];
        if let Some(last) = initial.last_mut() {
            *last >>= (64 - width % 64) % 64;
        }
        for id in 0..count {
            initial[id / 64] &= !(1u64 << (id % 64));
        }
        let mut masks = vec![initial.clone(); 256];
        for (id, pattern) in patterns.iter().enumerate() {
            for (position, &byte) in pattern.iter().enumerate() {
                let bit = (position + 1) * count + id;
                masks[usize::from(byte)][bit / 64] &= !(1u64 << (bit % 64));
            }
        }
        Self {
            masks,
            initial,
            lengths: patterns.iter().map(Vec::len).collect(),
            width,
        }
    }

    pub fn state_width(&self) -> usize {
        self.width
    }
    pub fn word_count(&self) -> usize {
        self.initial.len()
    }
    pub fn initial_state(&self) -> &[u64] {
        &self.initial
    }
    pub fn mask(&self, byte: u8) -> &[u64] {
        &self.masks[usize::from(byte)]
    }
    pub fn accepting_bit(&self, pattern_id: usize) -> usize {
        self.lengths[pattern_id] * self.lengths.len() + pattern_id
    }

    fn walk(&self, input: &[u8], mut observe: impl FnMut(usize, u8, &[u64])) {
        let count = self.lengths.len();
        if count == 0 {
            return;
        }
        let mut state = self.initial.clone();
        let word_shift = count / 64;
        let bit_shift = count % 64;
        for (offset, &byte) in input.iter().enumerate() {
            let mask = self.mask(byte);
            // Descending order preserves source words during the in-place shift.
            for dest in (0..state.len()).rev() {
                let mut shifted = 0;
                if dest >= word_shift {
                    let source = dest - word_shift;
                    shifted = state[source] << bit_shift;
                    if bit_shift != 0 && source > 0 {
                        shifted |= state[source - 1] >> (64 - bit_shift);
                    }
                }
                state[dest] = shifted | mask[dest];
            }
            let last = state.len() - 1;
            state[last] &= u64::MAX >> ((64 - self.width % 64) % 64);
            observe(offset + 1, byte, &state);
        }
    }

    fn report_matches(&self, end: usize, state: &[u64], mut report: impl FnMut(Match)) {
        for (id, &length) in self.lengths.iter().enumerate() {
            let bit = self.accepting_bit(id);
            if state[bit / 64] & (1u64 << (bit % 64)) == 0 {
                report(Match {
                    pattern_id: id,
                    start: end - length,
                    end,
                });
            }
        }
    }

    /// Reports overlaps and duplicate patterns, ordered by end offset then ID.
    /// Each call starts from the initial state.
    pub fn scan(&self, input: &[u8], mut report: impl FnMut(Match)) {
        self.walk(input, |end, _, state| {
            self.report_matches(end, state, &mut report)
        });
    }

    pub fn find_all(&self, input: &[u8]) -> Vec<Match> {
        let mut matches = Vec::new();
        self.scan(input, |m| matches.push(m));
        matches
    }

    /// Allocates mask and state snapshots separately from normal searching.
    pub fn trace(&self, input: &[u8]) -> Vec<TraceStep> {
        let mut steps = Vec::new();
        self.walk(input, |end, byte, state| {
            let mut matches = Vec::new();
            self.report_matches(end, state, |m| matches.push(m));
            steps.push(TraceStep {
                end,
                byte,
                mask: self.mask(byte).to_vec(),
                state: state.to_vec(),
                matches,
            });
        });
        steps
    }
}
