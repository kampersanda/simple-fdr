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
    filter::{Filter, SearchStats},
    generate::{self, Config, Dataset},
    multiple_shift_or::MultipleShiftOr,
    naive::Naive,
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
        /// Maximum number of balanced consecutive-ID buckets (grouping modes).
        #[arg(long, default_value = "2")]
        bucket_count: NonZeroUsize,
        /// Maximum per-pattern suffix length (truncation modes).
        #[arg(long, default_value = "2")]
        filter_length: NonZeroUsize,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Algorithm {
    Naive,
    MultipleShiftOr,
    Grouping,
    Truncation,
    GroupingTruncation,
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
    algorithm: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    state_width_bits: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state_words: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<FilterResult>,
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

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Generate(files) => write_json(&files.output, &load_dataset(&files.config)?),
        Command::Run {
            files,
            algorithm,
            bucket_count,
            filter_length,
        } => {
            let dataset = load_dataset(&files.config)?;
            // Generation and output serialization are excluded. This single sample
            // includes collecting matches; it is not a benchmark with repetitions.
            let started = Instant::now();
            let patterns = Patterns::new(dataset.patterns)?;
            let max_pattern_len = patterns.as_slice().iter().map(Vec::len).max().unwrap_or(0);
            let mut filter_result = None;
            let (construction_ns, search_ns, mut matches, name, width, words) = match algorithm {
                Algorithm::Naive => {
                    let matcher = Naive::new(patterns);
                    let construction_ns = started.elapsed().as_nanos();
                    let search = Instant::now();
                    let matches = matcher.find_all(&dataset.input);
                    (
                        construction_ns,
                        search.elapsed().as_nanos(),
                        matches,
                        "naive",
                        None,
                        None,
                    )
                }
                Algorithm::Grouping | Algorithm::Truncation | Algorithm::GroupingTruncation => {
                    let (name, buckets, length) = match algorithm {
                        Algorithm::Grouping => ("grouping", bucket_count, None),
                        Algorithm::Truncation => (
                            "truncation",
                            NonZeroUsize::new(patterns.as_slice().len().max(1)).unwrap(),
                            Some(filter_length),
                        ),
                        _ => ("grouping-truncation", bucket_count, Some(filter_length)),
                    };
                    let matcher = Filter::new(patterns, buckets, length);
                    let construction_ns = started.elapsed().as_nanos();
                    let search = Instant::now();
                    let matches = matcher.find_all(&dataset.input);
                    let search_ns = search.elapsed().as_nanos();
                    filter_result = Some(FilterResult {
                        requested_bucket_count: buckets.get(),
                        filter_length: length.map(NonZeroUsize::get),
                        buckets: matcher.buckets().to_vec(),
                        accepting_lengths: matcher.accepting_lengths().to_vec(),
                        statistics: matcher.statistics(&dataset.input),
                    });
                    (
                        construction_ns,
                        search_ns,
                        matches,
                        name,
                        Some(matcher.state_width()),
                        Some(matcher.word_count()),
                    )
                }
                Algorithm::MultipleShiftOr => {
                    let matcher = MultipleShiftOr::new(patterns);
                    let construction_ns = started.elapsed().as_nanos();
                    let search = Instant::now();
                    let matches = matcher.find_all(&dataset.input);
                    (
                        construction_ns,
                        search.elapsed().as_nanos(),
                        matches,
                        "multiple-shift-or",
                        Some(matcher.state_width()),
                        Some(matcher.word_count()),
                    )
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
                algorithm: name,
                state_width_bits: width,
                state_words: words,
                filter: filter_result,
                max_pattern_len,
                config: &dataset.config,
                input_len: dataset.input.len(),
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
