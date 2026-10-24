use std::{env, error::Error, fs, io, path::Path, process, time::Instant};

use serde::Serialize;
use simple_fdr::{
    Match, Patterns,
    generate::{self, Config},
    naive::Naive,
};

const USAGE: &str = "Usage:\n  simple-fdr generate CONFIG.json DATASET.json\n  simple-fdr run CONFIG.json RESULT.json";

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

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if args.len() != 3 || (args[0] != "generate" && args[0] != "run") {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, USAGE).into());
    }
    let config_path = Path::new(&args[1]);
    let output_path = Path::new(&args[2]);
    let config: Config = serde_json::from_slice(&fs::read(config_path)?)?;
    let dataset = generate::generate(&config)?;
    if args[0] == "generate" {
        write_json(output_path, &dataset)?;
    } else {
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
            config: &config,
            input_len: dataset.input.len(),
            pattern_count: config.pattern_count,
            construction_ns,
            search_ns,
            match_count: matches.len(),
            matches,
            planted_matches: dataset.planted_matches,
        };
        write_json(output_path, &result)?;
    }
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), Box<dyn Error>> {
    let mut json = serde_json::to_vec_pretty(value)?;
    json.push(b'\n');
    // Refuse to overwrite an existing config, dataset, or result accidentally.
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&json)?;
    Ok(())
}
