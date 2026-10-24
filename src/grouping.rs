//! Length-sorted contiguous partitioning with the paper's empirical cost.
use crate::Patterns;
use clap::ValueEnum;
use serde::Serialize;
use std::num::NonZeroUsize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    Balanced,
    LengthSorted,
    DynamicProgramming,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Coefficients {
    alpha: f64,
    beta: f64,
}

#[derive(Debug, Error)]
#[error("alpha must be finite and positive; beta must be finite and nonnegative")]
pub struct InvalidCoefficients;

impl Coefficients {
    pub fn new(alpha: f64, beta: f64) -> Result<Self, InvalidCoefficients> {
        if !alpha.is_finite() || alpha <= 0.0 || !beta.is_finite() || beta < 0.0 {
            return Err(InvalidCoefficients);
        }
        Ok(Self { alpha, beta })
    }

    pub fn alpha(self) -> f64 {
        self.alpha
    }
    pub fn beta(self) -> f64 {
        self.beta
    }
}

impl Default for Coefficients {
    fn default() -> Self {
        Self {
            alpha: 1.05,
            beta: 3.0,
        }
    }
}

/// Exactly min(requested, pattern_count) nonempty buckets; length ties retain IDs.
/// DP minimizes sum(size^alpha / min_len^beta) over length-sorted contiguous
/// partitions, using original lengths rather than truncated lengths.
pub fn partition(
    patterns: &Patterns,
    requested: NonZeroUsize,
    strategy: Strategy,
    coefficients: Coefficients,
) -> Vec<Vec<usize>> {
    let n = patterns.as_slice().len();
    let q = requested.get().min(n);
    if q == 0 {
        return Vec::new();
    }
    let mut ids: Vec<_> = (0..n).collect();
    if strategy != Strategy::Balanced {
        ids.sort_by_key(|&id| patterns.as_slice()[id].len());
    }
    if strategy != Strategy::DynamicProgramming || q == 1 || q == n {
        let mut start = 0;
        return (0..q)
            .map(|bucket| {
                let size = n / q + usize::from(bucket < n % q);
                let members = ids[start..start + size].to_vec();
                start += size;
                members
            })
            .collect();
    }
    // Scale every cost by min_len^beta / n^alpha to avoid overflow.
    // No powf calls in the O(q*n^2) transition loop.
    let sizes: Vec<_> = (0..=n)
        .map(|size| (size as f64 / n as f64).powf(coefficients.alpha))
        .collect();
    let min_len = patterns.as_slice()[ids[0]].len() as f64;
    let weights: Vec<_> = ids
        .iter()
        .map(|&id| (min_len / patterns.as_slice()[id].len() as f64).powf(coefficients.beta))
        .collect();
    let mut previous = vec![f64::INFINITY; n + 1];
    previous[0] = 0.0;
    let mut splits = vec![vec![0; n + 1]; q + 1];
    for (buckets, row) in splits.iter_mut().enumerate().take(q + 1).skip(1) {
        let mut current = vec![f64::INFINITY; n + 1];
        for end in buckets..=n - (q - buckets) {
            for start in buckets - 1..end {
                let cost = previous[start] + sizes[end - start] * weights[start];
                if cost < current[end] {
                    current[end] = cost;
                    row[end] = start;
                }
            }
        }
        previous = current;
    }
    let mut buckets = Vec::with_capacity(q);
    let mut end = n;
    for row in splits.iter().skip(1).rev() {
        let start = row[end];
        buckets.push(ids[start..end].to_vec());
        end = start;
    }
    buckets.reverse();
    buckets
}
