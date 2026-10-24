//! Measure every technique and the FDR options with the same input and result sink.
//!
//! `techniques` times each matcher of the article, then FDR with one option
//! changed at a time. `comparison` sweeps super-character bits, batch sizes,
//! verifiers and available backends. Every configuration is checked against
//! the reference matcher, and FDR candidate lists are checked against the
//! integer sequential filter with the same bits.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use simple_fdr::{
    Match, Patterns,
    fdr_filter::{BatchSize, FdrFilter, SuperCharacterBits, Verification},
    filter::Filter,
    generate::DatasetConfig,
    grouping::{Coefficients, Strategy},
    multiple_shift_or::MultipleShiftOr,
    naive::Naive,
    simd::Backend,
};
use std::{env, fs, hint::black_box, num::NonZeroUsize, path::Path, time::Instant};

const WARMUPS: usize = 3;
const REPETITIONS: usize = 9;
const BUCKET_COUNT: usize = 8;
const FILTER_LENGTH: usize = 8;

/// Time `scan`, which must return the number of reported matches.
fn time(expected: usize, scan: impl Fn() -> usize) -> Value {
    let mut samples = Vec::new();
    for iteration in 0..WARMUPS + REPETITIONS {
        let started = Instant::now();
        let count = scan();
        let elapsed = started.elapsed().as_nanos();
        assert_eq!(black_box(count), expected);
        if iteration >= WARMUPS {
            samples.push(elapsed);
        }
    }
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    json!({ "search_samples_ns": samples, "median_search_ns": sorted[sorted.len() / 2] })
}

/// Count matches without storing them.
fn count(scan: impl FnOnce(&mut dyn FnMut(Match))) -> usize {
    let mut count = 0;
    scan(&mut |m| {
        black_box(m);
        count += 1;
    });
    count
}

fn check(mut matches: Vec<Match>, expected: &[Match]) {
    matches.sort_unstable();
    assert_eq!(matches, expected);
}

fn native_backend() -> Backend {
    [Backend::Neon, Backend::Sse2]
        .into_iter()
        .find(|backend| backend.is_supported())
        .unwrap_or(Backend::Integer)
}

fn fdr(
    patterns: &Patterns,
    strategy: Strategy,
    size: BatchSize,
    bits: SuperCharacterBits,
    verification: Verification,
    backend: Backend,
) -> Result<(FdrFilter, u128)> {
    let started = Instant::now();
    let filter = FdrFilter::new(
        patterns.clone(),
        NonZeroUsize::new(BUCKET_COUNT).unwrap(),
        NonZeroUsize::new(FILTER_LENGTH).unwrap(),
        strategy,
        Coefficients::default(),
    )
    .with_batch_size(size)
    .with_super_character(bits)
    .with_verification(verification)
    .with_backend(backend)
    .map_err(anyhow::Error::msg)?;
    Ok((filter, started.elapsed().as_nanos()))
}

fn fdr_entry(filter: &FdrFilter, construction_ns: u128, input: &[u8], expected: &[Match]) -> Value {
    check(filter.find_all(input), expected);
    let mut entry = json!({
        "super_character_bits": filter.super_character_bits().get(),
        "batch_size": filter.batch_size().get(),
        "verification": filter.verification(),
        "backend": filter.backend(),
        "construction_ns": construction_ns,
        "state_width_bits": filter.state_width(),
        "state_words": filter.word_count(),
        "mask_table_bytes": filter.mask_table_bytes(),
        "verification_table_bytes": filter.verification_table_bytes(),
        "effective_lengths": filter.effective_lengths(),
        "statistics": filter.statistics(input),
    });
    merge(
        &mut entry,
        time(expected.len(), || {
            count(|report| filter.scan(black_box(input), report))
        }),
    );
    entry
}

fn merge(entry: &mut Value, other: Value) {
    let (Value::Object(entry), Value::Object(other)) = (entry, other) else {
        unreachable!()
    };
    entry.extend(other);
}

fn techniques(patterns: &Patterns, input: &[u8], expected: &[Match]) -> Result<Vec<Value>> {
    let mut results = Vec::new();
    let buckets = NonZeroUsize::new(BUCKET_COUNT).unwrap();
    let length = NonZeroUsize::new(FILTER_LENGTH).unwrap();

    let naive = Naive::new(patterns.clone());
    let mut entry = json!({ "name": "naive" });
    merge(
        &mut entry,
        time(expected.len(), || {
            count(|r| naive.scan(black_box(input), r))
        }),
    );
    results.push(entry);

    let started = Instant::now();
    let matcher = MultipleShiftOr::new(patterns.clone());
    let construction_ns = started.elapsed().as_nanos();
    check(matcher.find_all(input), expected);
    let mut entry = json!({
        "name": "multiple-shift-or",
        "construction_ns": construction_ns,
        "state_width_bits": matcher.state_width(),
        "state_words": matcher.word_count(),
    });
    merge(
        &mut entry,
        time(expected.len(), || {
            count(|r| matcher.scan(black_box(input), r))
        }),
    );
    results.push(entry);

    let one_per_pattern = NonZeroUsize::new(patterns.as_slice().len().max(1)).unwrap();
    for (name, bucket_count, filter_length) in [
        ("grouping", buckets, None),
        ("truncation", one_per_pattern, Some(length)),
        ("grouping-truncation", buckets, Some(length)),
    ] {
        let started = Instant::now();
        let filter = Filter::new(patterns.clone(), bucket_count, filter_length);
        let construction_ns = started.elapsed().as_nanos();
        check(filter.find_all(input), expected);
        let mut entry = json!({
            "name": name,
            "construction_ns": construction_ns,
            "state_width_bits": filter.state_width(),
            "state_words": filter.word_count(),
            "statistics": filter.statistics(input),
        });
        merge(
            &mut entry,
            time(expected.len(), || {
                count(|r| filter.scan(black_box(input), r))
            }),
        );
        results.push(entry);
    }

    use {BatchSize as B, SuperCharacterBits as S, Verification as V};
    let simd = native_backend();
    for (name, strategy, size, bits, verification, backend) in [
        (
            "fdr-balanced",
            Strategy::Balanced,
            B::One,
            S::Zero,
            V::Exhaustive,
            Backend::Integer,
        ),
        (
            "fdr-length-sorted",
            Strategy::LengthSorted,
            B::One,
            S::Zero,
            V::Exhaustive,
            Backend::Integer,
        ),
        (
            "fdr-sequential",
            Strategy::DynamicProgramming,
            B::One,
            S::Zero,
            V::Exhaustive,
            Backend::Integer,
        ),
        (
            "fdr-super-character-4",
            Strategy::DynamicProgramming,
            B::One,
            S::Four,
            V::Exhaustive,
            Backend::Integer,
        ),
        (
            "fdr-hash",
            Strategy::DynamicProgramming,
            B::One,
            S::Zero,
            V::Hash,
            Backend::Integer,
        ),
        (
            "fdr-batch-8",
            Strategy::DynamicProgramming,
            B::Eight,
            S::Zero,
            V::Exhaustive,
            Backend::Integer,
        ),
        (
            "fdr-batch-8-simd",
            Strategy::DynamicProgramming,
            B::Eight,
            S::Zero,
            V::Exhaustive,
            simd,
        ),
        (
            "fdr-all",
            Strategy::DynamicProgramming,
            B::Eight,
            S::Four,
            V::Hash,
            simd,
        ),
    ] {
        let (filter, construction_ns) = fdr(patterns, strategy, size, bits, verification, backend)?;
        let mut entry =
            json!({ "name": name, "grouping_strategy": strategy, "buckets": filter.buckets() });
        merge(
            &mut entry,
            fdr_entry(&filter, construction_ns, input, expected),
        );
        results.push(entry);
    }
    Ok(results)
}

fn comparison(patterns: &Patterns, input: &[u8], expected: &[Match]) -> Result<Vec<Value>> {
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
                    let (filter, construction_ns) = fdr(
                        patterns,
                        Strategy::DynamicProgramming,
                        size,
                        bits,
                        verification,
                        backend,
                    )?;
                    let mut candidates = Vec::new();
                    filter.scan_candidates(input, |c| candidates.push(c));
                    assert_eq!(
                        &candidates,
                        reference_candidates.get_or_insert_with(|| candidates.clone())
                    );
                    results.push(fdr_entry(&filter, construction_ns, input, expected));
                }
            }
        }
    }
    Ok(results)
}

fn main() -> Result<()> {
    let path = env::args().nth(1).context("usage: bench CONFIG.json")?;
    let path = Path::new(&path);
    let config: DatasetConfig = serde_json::from_slice(&fs::read(path)?)?;
    let data = config.build(path.parent().unwrap_or(Path::new("")))?;
    let input = &data.input;
    let patterns = Patterns::new(data.patterns)?;
    let mut expected = Naive::new(patterns.clone()).find_all(input);
    expected.sort_unstable();

    let techniques = techniques(&patterns, input, &expected)?;
    let comparison = comparison(&patterns, input, &expected)?;
    let lengths = patterns.as_slice().iter().map(Vec::len);
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
            "bucket_count": BUCKET_COUNT,
            "filter_length": FILTER_LENGTH,
            "comparison_strategy": Strategy::DynamicProgramming,
            "coefficients": Coefficients::default(),
            "input_len": input.len(),
            "pattern_count": patterns.as_slice().len(),
            "min_pattern_len": lengths.clone().min(),
            "max_pattern_len": lengths.max(),
            "match_count": expected.len(),
            "techniques": techniques,
            "comparison": comparison,
        }))?
    );
    Ok(())
}
