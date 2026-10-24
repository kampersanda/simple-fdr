//! Measure FDR options with the same input, partition and result sink.
//!
//! Sweeps super-character bits, batch sizes, verifiers and available backends.
//! Every configuration is checked against the reference matcher, and candidate
//! lists are checked against the integer sequential filter with the same bits.
use anyhow::{Context, Result};
use serde_json::json;
use simple_fdr::{
    Patterns,
    fdr_filter::{BatchSize, FdrFilter, SuperCharacterBits, Verification},
    generate::{self, Config},
    grouping::{Coefficients, Strategy},
    naive::Naive,
    simd::Backend,
};
use std::{env, fs, hint::black_box, num::NonZeroUsize, time::Instant};

const WARMUPS: usize = 3;
const REPETITIONS: usize = 9;

fn main() -> Result<()> {
    let path = env::args().nth(1).context("usage: bench CONFIG.json")?;
    let config: Config = serde_json::from_slice(&fs::read(path)?)?;
    let data = generate::generate(&config)?;
    let input = &data.input;
    let patterns = Patterns::new(data.patterns)?;
    let expected = Naive::new(patterns.clone()).find_all(input);
    let eight = NonZeroUsize::new(8).unwrap();

    let mut results = Vec::new();
    for bits in [
        SuperCharacterBits::Zero,
        SuperCharacterBits::Four,
        SuperCharacterBits::Eight,
    ] {
        let mut reference_candidates = None;
        for size in [
            BatchSize::One,
            BatchSize::Two,
            BatchSize::Four,
            BatchSize::Eight,
        ] {
            for verification in [Verification::Exhaustive, Verification::Hash] {
                for backend in [Backend::Integer, Backend::Neon, Backend::Sse2] {
                    if !backend.is_supported() {
                        continue;
                    }
                    let started = Instant::now();
                    let filter = FdrFilter::new(
                        patterns.clone(),
                        eight,
                        eight,
                        Strategy::DynamicProgramming,
                        Coefficients::default(),
                    )
                    .with_batch_size(size)
                    .with_super_character(bits)
                    .with_verification(verification)
                    .with_backend(backend)
                    .map_err(anyhow::Error::msg)?;
                    let construction_ns = started.elapsed().as_nanos();

                    let mut matches = filter.find_all(input);
                    matches.sort_unstable();
                    assert_eq!(matches, expected);
                    let mut candidates = Vec::new();
                    filter.scan_candidates(input, |c| candidates.push(c));
                    assert_eq!(
                        &candidates,
                        reference_candidates.get_or_insert_with(|| candidates.clone())
                    );

                    let mut samples = Vec::new();
                    for iteration in 0..WARMUPS + REPETITIONS {
                        let started = Instant::now();
                        let mut count = 0;
                        filter.scan(black_box(input), |m| {
                            black_box(m);
                            count += 1;
                        });
                        let elapsed = started.elapsed().as_nanos();
                        assert_eq!(black_box(count), expected.len());
                        if iteration >= WARMUPS {
                            samples.push(elapsed);
                        }
                    }
                    let mut sorted = samples.clone();
                    sorted.sort_unstable();
                    let median = sorted[sorted.len() / 2];
                    results.push(json!({
                        "super_character_bits": bits.get(),
                        "batch_size": size.get(),
                        "verification": verification,
                        "backend": backend,
                        "construction_ns": construction_ns,
                        "state_width_bits": filter.state_width(),
                        "state_words": filter.word_count(),
                        "mask_table_bytes": filter.mask_table_bytes(),
                        "verification_table_bytes": filter.verification_table_bytes(),
                        "search_samples_ns": samples,
                        "median_search_ns": median,
                        "mib_per_second": input.len() as f64 * 1e9 / median as f64 / 1048576.0,
                        "statistics": filter.statistics(input),
                    }));
                }
            }
        }
    }
    let filter = FdrFilter::new(
        patterns,
        eight,
        eight,
        Strategy::DynamicProgramming,
        Coefficients::default(),
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "config": config,
            "generator_version": data.generator_version,
            "architecture": env::consts::ARCH,
            "os": env::consts::OS,
            "debug_assertions": cfg!(debug_assertions),
            "warmups": WARMUPS,
            "repetitions": REPETITIONS,
            "aggregation": "median",
            "result_sink": "black_box match and count; no allocation or sorting",
            "bucket_count": 8,
            "filter_length": 8,
            "strategy": "dynamic-programming",
            "coefficients": Coefficients::default(),
            "buckets": filter.buckets(),
            "effective_lengths": filter.effective_lengths(),
            "match_count": expected.len(),
            "comparison": results,
        }))?
    );
    Ok(())
}
