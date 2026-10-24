use std::{
    env, fs,
    io::Write,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use simple_fdr::{
    Match, Patterns,
    fdr_filter::{BatchSize, FdrFilter, SuperCharacterBits, Verification},
    filter::{Filter, SearchStats},
    generate::{self, Config, Dataset},
    grouping::{Coefficients, Strategy},
    multiple_shift_or::MultipleShiftOr,
    naive::Naive,
    simd::Backend,
};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate a reproducible dataset and save it as JSON.
    Generate(Files),
    /// Run a matcher and save results as JSON.
    Run {
        #[command(flatten)]
        files: Files,
        #[arg(long, value_enum, default_value_t = Algorithm::Naive)]
        algorithm: Algorithm,
        /// Maximum number of buckets (grouping and FDR modes).
        #[arg(long, default_value = "2")]
        bucket_count: NonZeroUsize,
        /// Maximum suffix length (truncation and FDR modes).
        #[arg(long, default_value = "2")]
        filter_length: NonZeroUsize,
        /// Input bytes per FDR batch; fdr-sequential always uses 1.
        #[arg(long, value_enum, default_value = "8")]
        batch_size: BatchSize,
        /// Low bits of the next byte included in FDR masks.
        #[arg(long, value_enum, default_value = "0")]
        super_character_bits: SuperCharacterBits,
        /// FDR mask accumulation backend.
        #[arg(long, value_enum, default_value = "integer")]
        backend: Backend,
        /// Candidate verifier for FDR modes.
        #[arg(long, value_enum, default_value = "exhaustive")]
        verification: Verification,
        /// Bucket partition for the FDR filter.
        #[arg(long, value_enum, default_value = "dynamic-programming")]
        grouping_strategy: Strategy,
        #[arg(long, default_value = "1.05")]
        alpha: f64,
        #[arg(long, default_value = "3")]
        beta: f64,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum Algorithm {
    Naive,
    MultipleShiftOr,
    Grouping,
    Truncation,
    GroupingTruncation,
    FdrSequential,
    FdrBatched,
}

#[derive(Args)]
struct Files {
    /// Generator configuration (JSON).
    config: PathBuf,
    /// Output JSON file (must not already exist).
    output: PathBuf,
}

#[derive(Serialize)]
struct ResultFile<'a> {
    format_version: u32,
    generator_version: u32,
    package_version: &'static str,
    architecture: &'static str,
    os: &'static str,
    debug_assertions: bool,
    algorithm: Algorithm,
    #[serde(skip_serializing_if = "Option::is_none")]
    state_width_bits: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state_words: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<FilterResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fdr_filter: Option<FdrFilterResult>,
    max_pattern_len: usize,
    config: &'a Config,
    input_len: usize,
    pattern_count: usize,
    construction_ns: u128,
    search_ns: u128,
    match_count: usize,
    matches: Vec<Match>,
    planted_matches: Vec<Match>,
}

#[derive(Serialize)]
struct FilterResult {
    requested_bucket_count: usize,
    filter_length: Option<usize>,
    buckets: Vec<Vec<usize>>,
    accepting_lengths: Vec<Vec<usize>>,
    #[serde(flatten)]
    statistics: SearchStats,
}

#[derive(Serialize)]
struct FdrFilterResult {
    backend: Backend,
    verification: Verification,
    verification_table_bytes: usize,
    super_character_bits: usize,
    batch_size: usize,
    requested_bucket_count: usize,
    filter_length: usize,
    grouping_strategy: Strategy,
    coefficients: Coefficients,
    buckets: Vec<Vec<usize>>,
    effective_lengths: Vec<usize>,
    mask_table_bytes: usize,
    #[serde(flatten)]
    statistics: SearchStats,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Generate(files) => write_json(&files.output, &load_dataset(&files.config)?),
        Command::Run {
            files,
            algorithm,
            bucket_count,
            filter_length,
            grouping_strategy,
            batch_size,
            super_character_bits,
            verification,
            backend,
            alpha,
            beta,
        } => {
            let coefficients = Coefficients::new(alpha, beta)?;
            let dataset = load_dataset(&files.config)?;
            let input = &dataset.input;
            // Generation and output serialization are excluded. This single sample
            // includes collecting matches; it is not a benchmark with repetitions.
            let started = Instant::now();
            let patterns = Patterns::new(dataset.patterns)?;
            let max_pattern_len = patterns.as_slice().iter().map(Vec::len).max().unwrap_or(0);
            let (mut construction_ns, mut search_ns) = (0, 0);
            let mut timed = |find_all: &dyn Fn() -> Vec<Match>| {
                construction_ns = started.elapsed().as_nanos();
                let search = Instant::now();
                let matches = find_all();
                search_ns = search.elapsed().as_nanos();
                matches
            };
            let mut filter_result = None;
            let mut fdr_filter_result = None;
            let (mut matches, state) = match algorithm {
                Algorithm::Naive => {
                    let matcher = Naive::new(patterns);
                    (timed(&|| matcher.find_all(input)), None)
                }
                Algorithm::MultipleShiftOr => {
                    let matcher = MultipleShiftOr::new(patterns);
                    let matches = timed(&|| matcher.find_all(input));
                    (matches, Some((matcher.state_width(), matcher.word_count())))
                }
                Algorithm::Grouping | Algorithm::Truncation | Algorithm::GroupingTruncation => {
                    let (buckets, length) = match algorithm {
                        Algorithm::Grouping => (bucket_count, None),
                        Algorithm::Truncation => (
                            NonZeroUsize::new(patterns.as_slice().len().max(1)).unwrap(),
                            Some(filter_length),
                        ),
                        _ => (bucket_count, Some(filter_length)),
                    };
                    let matcher = Filter::new(patterns, buckets, length);
                    let matches = timed(&|| matcher.find_all(input));
                    filter_result = Some(FilterResult {
                        requested_bucket_count: buckets.get(),
                        filter_length: length.map(NonZeroUsize::get),
                        buckets: matcher.buckets().to_vec(),
                        accepting_lengths: matcher.accepting_lengths().to_vec(),
                        statistics: matcher.statistics(input),
                    });
                    (matches, Some((matcher.state_width(), matcher.word_count())))
                }
                Algorithm::FdrSequential | Algorithm::FdrBatched => {
                    let batch_size = match algorithm {
                        Algorithm::FdrSequential => BatchSize::One,
                        _ => batch_size,
                    };
                    let matcher = FdrFilter::new(
                        patterns,
                        bucket_count,
                        filter_length,
                        grouping_strategy,
                        coefficients,
                    )
                    .with_batch_size(batch_size)
                    .with_super_character(super_character_bits)
                    .with_verification(verification)
                    .with_backend(backend)
                    .map_err(anyhow::Error::msg)?;
                    let matches = timed(&|| matcher.find_all(input));
                    fdr_filter_result = Some(FdrFilterResult {
                        backend,
                        verification,
                        verification_table_bytes: matcher.verification_table_bytes(),
                        super_character_bits: super_character_bits.get(),
                        batch_size: batch_size.get(),
                        requested_bucket_count: bucket_count.get(),
                        filter_length: filter_length.get(),
                        grouping_strategy,
                        coefficients,
                        buckets: matcher.buckets().to_vec(),
                        effective_lengths: matcher.effective_lengths().to_vec(),
                        mask_table_bytes: matcher.mask_table_bytes(),
                        statistics: matcher.statistics(input),
                    });
                    (matches, Some((matcher.state_width(), matcher.word_count())))
                }
            };
            // Normalize output order outside the search measurement.
            matches.sort_unstable();
            let result = ResultFile {
                format_version: 1,
                generator_version: dataset.generator_version,
                package_version: env!("CARGO_PKG_VERSION"),
                architecture: env::consts::ARCH,
                os: env::consts::OS,
                debug_assertions: cfg!(debug_assertions),
                algorithm,
                state_width_bits: state.map(|(bits, _)| bits),
                state_words: state.map(|(_, words)| words),
                filter: filter_result,
                fdr_filter: fdr_filter_result,
                max_pattern_len,
                config: &dataset.config,
                input_len: input.len(),
                pattern_count: dataset.config.pattern_count,
                construction_ns,
                search_ns,
                match_count: matches.len(),
                matches,
                planted_matches: dataset.planted_matches,
            };
            write_json(&files.output, &result)
        }
    }
}

fn load_dataset(path: &Path) -> Result<Dataset> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read config {}", path.display()))?;
    let config = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    Ok(generate::generate(&config)?)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut json = serde_json::to_vec_pretty(value)?;
    json.push(b'\n');
    let mut file = fs::File::create_new(path)
        .with_context(|| format!("failed to create output {}", path.display()))?;
    file.write_all(&json)
        .with_context(|| format!("failed to write output {}", path.display()))?;
    Ok(())
}
