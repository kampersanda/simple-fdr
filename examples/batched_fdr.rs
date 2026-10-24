//! Warmup/repeated comparison with identical masks, verifier and result sink.
use anyhow::{Context, Result};
use serde_json::json;
use simple_fdr::{
    Patterns,
    fdr_filter::{BatchSize, FdrFilter},
    generate::{self, Config},
    grouping::{Coefficients, Strategy},
    naive::Naive,
};
use std::{env, fs, hint::black_box, num::NonZeroUsize, time::Instant};

fn main() -> Result<()> {
    let path = env::args()
        .nth(1)
        .context("usage: batched_fdr CONFIG.json")?;
    let config: Config = serde_json::from_slice(&fs::read(path)?)?;
    let data = generate::generate(&config)?;
    let patterns = Patterns::new(data.patterns)?;
    let expected = Naive::new(patterns.clone()).find_all(&data.input);
    let mut reference_candidates = Vec::new();
    let mut results = Vec::new();
    for size in [
        BatchSize::One,
        BatchSize::Two,
        BatchSize::Four,
        BatchSize::Eight,
    ] {
        let started = Instant::now();
        let filter = FdrFilter::new(
            patterns.clone(),
            NonZeroUsize::new(8).unwrap(),
            NonZeroUsize::new(8).unwrap(),
            Strategy::DynamicProgramming,
            Coefficients::default(),
        )
        .with_batch_size(size);
        let construction_ns = started.elapsed().as_nanos();
        let mut matches = filter.find_all(&data.input);
        matches.sort_unstable();
        assert_eq!(matches, expected);
        let mut candidates = Vec::new();
        filter.scan_candidates(&data.input, |c| candidates.push(c));
        if size == BatchSize::One {
            reference_candidates = candidates;
        } else {
            assert_eq!(candidates, reference_candidates);
        }
        let mut samples = Vec::new();
        for iteration in 0..12 {
            let started = Instant::now();
            let mut count = 0;
            filter.scan(black_box(&data.input), |m| {
                black_box(m);
                count += 1;
            });
            let elapsed = started.elapsed().as_nanos();
            assert_eq!(black_box(count), expected.len());
            if iteration >= 3 {
                samples.push(elapsed);
            }
        }
        let mut sorted = samples.clone();
        sorted.sort_unstable();
        let median = sorted[sorted.len() / 2];
        results.push(json!({"batch_size": size.get(), "construction_ns": construction_ns,
            "state_width_bits": filter.state_width(), "state_words": filter.word_count(),
            "mask_table_bytes": filter.mask_table_bytes(), "search_samples_ns": samples,
            "median_search_ns": median, "mib_per_second": data.input.len() as f64 * 1e9 / median as f64 / 1048576.0,
            "statistics": filter.statistics(&data.input), "match_count": expected.len(),
            "buckets": filter.buckets(), "effective_lengths": filter.effective_lengths()}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "config": config, "generator_version": data.generator_version,
            "architecture": env::consts::ARCH, "os": env::consts::OS,
            "debug_assertions": cfg!(debug_assertions), "warmups": 3, "repetitions": 9,
            "aggregation": "median", "result_sink": "black_box match and count; no allocation or sorting",
            "bucket_count": 8, "filter_length": 8, "strategy": "dynamic-programming",
            "alpha": 1.05, "beta": 3, "comparison": results,
        }))?
    );
    Ok(())
}
