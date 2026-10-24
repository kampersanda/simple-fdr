use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use simple_fdr::{
    Match, Patterns,
    generate::{self, Config, Dataset},
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
    /// Run the reference matcher and save results as JSON.
    Run(Files),
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
    config: &'a Config,
    input_len: usize,
    pattern_count: usize,
    construction_ns: u128,
    search_ns: u128,
    match_count: usize,
    matches: Vec<Match>,
    planted_matches: Vec<Match>,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Generate(files) => write_json(&files.output, &load_dataset(&files.config)?),
        Command::Run(files) => {
            let dataset = load_dataset(&files.config)?;
            // Generation and output serialization are excluded. This single sample
            // includes collecting matches; it is not a benchmark with repetitions.
            let started = Instant::now();
            let matcher = Naive::new(Patterns::new(dataset.patterns)?);
            let construction_ns = started.elapsed().as_nanos();
            let started = Instant::now();
            let matches = matcher.find_all(&dataset.input);
            let search_ns = started.elapsed().as_nanos();
            let result = ResultFile {
                format_version: 1,
                generator_version: dataset.generator_version,
                package_version: env!("CARGO_PKG_VERSION"),
                architecture: env::consts::ARCH,
                os: env::consts::OS,
                debug_assertions: cfg!(debug_assertions),
                algorithm: "naive",
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
