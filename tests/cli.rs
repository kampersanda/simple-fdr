use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use simple_fdr::{
    Patterns,
    generate::{Dataset, generate},
    naive::Naive,
};

fn cli(command: &str, config: &Path, output: &Path) -> Output {
    run(command, config, output, &[])
}

fn run(command: &str, config: &Path, output: &Path, options: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_simple-fdr"))
        .arg(command)
        .arg(config)
        .arg(output)
        .args(options)
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn saves_reproducible_data_and_complete_experiment_results() {
    let directory = tempfile::tempdir().unwrap();
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("experiments/small.json");
    let dataset_path = directory.path().join("dataset.json");
    let result_path = directory.path().join("result.json");

    let output = cli("generate", &config, &dataset_path);
    assert_success(&output);
    let data: Dataset = read_json(&dataset_path);
    assert_eq!(data, generate(&data.config).unwrap());

    let output = cli("run", &config, &result_path);
    assert_success(&output);
    let result: serde_json::Value = read_json(&result_path);
    let expected = Naive::new(Patterns::new(data.patterns).unwrap()).find_all(&data.input);
    assert_eq!(result["format_version"], 1);
    assert_eq!(result["generator_version"], 2);
    assert_eq!(result["algorithm"], "naive");
    assert_eq!(
        result["config"],
        serde_json::to_value(&data.config).unwrap()
    );
    assert_eq!(result["matches"], serde_json::to_value(&expected).unwrap());
    assert_eq!(result["match_count"], expected.len());
    assert_eq!(
        result["planted_matches"],
        serde_json::to_value(&data.planted_matches).unwrap()
    );
    assert!(result["construction_ns"].is_number());
    assert!(result["search_ns"].is_number());

    let before = fs::read(&result_path).unwrap();
    assert!(!cli("run", &config, &result_path).status.success());
    assert_eq!(before, fs::read(&result_path).unwrap());
}

#[test]
fn invalid_config_fails_without_creating_a_result() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.json");
    let result = directory.path().join("result.json");
    fs::write(
        &config,
        r#"{
        "seed": 1, "input_len": 10, "pattern_count": 1,
        "min_pattern_len": 0, "max_pattern_len": 3
    }"#,
    )
    .unwrap();
    assert!(!cli("run", &config, &result).status.success());
    assert!(!result.exists());
}

#[test]
fn help_succeeds_and_unknown_commands_fail() {
    for args in [
        vec!["--help"],
        vec!["--version"],
        vec!["generate", "--help"],
        vec!["run", "--help"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_simple-fdr"))
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
    }
    assert!(
        !cli("unknown", Path::new("unused"), Path::new("unused"))
            .status
            .success()
    );
}

#[test]
fn missing_or_extra_arguments_fail_before_reading_config() {
    for args in [
        vec![],
        vec!["generate"],
        vec!["run", "unused"],
        vec!["run", "unused", "unused", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_simple-fdr"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
    }
}

#[test]
fn file_errors_name_the_config_path() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("missing.json");
    let result = directory.path().join("result.json");
    let output = cli("run", &config, &result);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("failed to read config"));
    assert!(error.contains(config.to_str().unwrap()));
    assert!(!result.exists());
}

#[test]
fn multiple_shift_or_saves_width_and_reference_matches() {
    let directory = tempfile::tempdir().unwrap();
    for (count, length) in [(3, 4), (100, 16), (65, 1), (0, 1)] {
        let config_path = directory.path().join(format!("config-{count}.json"));
        let result_path = directory.path().join(format!("result-{count}.json"));
        fs::write(
            &config_path,
            serde_json::to_vec(&serde_json::json!({
                "seed": 29, "input_len": 128, "pattern_count": count,
                "min_pattern_len": length, "max_pattern_len": length,
                "alphabet": [0, 1, 2, 3]
            }))
            .unwrap(),
        )
        .unwrap();
        let output = run(
            "run",
            &config_path,
            &result_path,
            &["--algorithm", "multiple-shift-or"],
        );
        assert_success(&output);
        let result: serde_json::Value = read_json(&result_path);
        let data = generate(&read_json(&config_path)).unwrap();
        let expected = Naive::new(Patterns::new(data.patterns).unwrap()).find_all(&data.input);
        let width: usize = count * (length + 1);
        assert_eq!(result["algorithm"], "multiple-shift-or");
        assert_eq!(result["state_width_bits"], width);
        assert_eq!(result["state_words"], width.div_ceil(64));
        assert_eq!(
            result["max_pattern_len"],
            if count == 0 { 0 } else { length }
        );
        assert_eq!(result["matches"], serde_json::to_value(expected).unwrap());
        assert!(result["construction_ns"].is_number());
        assert!(result["search_ns"].is_number());
    }
}

#[test]
fn filter_algorithms_save_reference_matches_configuration_and_statistics() {
    use simple_fdr::filter::Filter;
    use std::num::NonZeroUsize;
    let directory = tempfile::tempdir().unwrap();
    for count in [0, 3, 65] {
        let config_path = directory.path().join(format!("config-filter-{count}.json"));
        fs::write(
            &config_path,
            serde_json::to_vec(&serde_json::json!({
                "seed": 31, "input_len": 128, "pattern_count": count,
                "min_pattern_len": 1, "max_pattern_len": 8, "alphabet": [0, 1, 2, 3]
            }))
            .unwrap(),
        )
        .unwrap();
        let data = generate(&read_json(&config_path)).unwrap();
        let patterns = Patterns::new(data.patterns).unwrap();
        let expected = Naive::new(patterns.clone()).find_all(&data.input);
        for (name, buckets, length) in [
            ("grouping", 2, None),
            ("truncation", count.max(1), NonZeroUsize::new(2)),
            ("grouping-truncation", 2, NonZeroUsize::new(2)),
        ] {
            let result_path = directory.path().join(format!("{name}-{count}.json"));
            let output = run(
                "run",
                &config_path,
                &result_path,
                &[
                    "--algorithm",
                    name,
                    "--bucket-count",
                    "2",
                    "--filter-length",
                    "2",
                ],
            );
            assert_success(&output);
            let result: serde_json::Value = read_json(&result_path);
            let filter = Filter::new(
                patterns.clone(),
                NonZeroUsize::new(buckets).unwrap(),
                length,
            );
            let stats = filter.statistics(&data.input);
            assert_eq!(result["algorithm"], name);
            assert_eq!(result["matches"], serde_json::to_value(&expected).unwrap());
            assert_eq!(result["state_width_bits"], filter.state_width());
            assert_eq!(result["state_words"], filter.word_count());
            assert_eq!(result["filter"]["requested_bucket_count"], buckets);
            assert_eq!(
                result["filter"]["filter_length"],
                serde_json::to_value(length).unwrap()
            );
            assert_eq!(
                result["filter"]["buckets"],
                serde_json::to_value(filter.buckets()).unwrap()
            );
            assert_eq!(
                result["filter"]["accepting_lengths"],
                serde_json::to_value(filter.accepting_lengths()).unwrap()
            );
            assert_eq!(result["filter"]["candidate_count"], stats.candidate_count);
            assert_eq!(result["filter"]["full_comparisons"], stats.full_comparisons);
            assert_eq!(
                result["filter"]["false_positive_candidates"],
                stats.false_positive_candidates
            );
        }
    }
}

#[test]
fn zero_filter_parameters_fail_before_creating_output() {
    let directory = tempfile::tempdir().unwrap();
    for flag in ["--bucket-count", "--filter-length"] {
        let result = directory.path().join("result.json");
        let output = run(
            "run",
            Path::new("unused"),
            &result,
            &["--algorithm", "grouping-truncation", flag, "0"],
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(!result.exists());
    }
}

#[test]
fn fdr_sequential_saves_partitions_lengths_coefficients_and_matches() {
    use simple_fdr::{
        fdr_filter::FdrFilter,
        grouping::{Coefficients, Strategy},
    };
    use std::num::NonZeroUsize;
    let directory = tempfile::tempdir().unwrap();
    for count in [0, 3, 65] {
        let config = directory.path().join(format!("fdr-{count}.json"));
        fs::write(
            &config,
            serde_json::to_vec(&serde_json::json!({
                "seed": 41, "input_len": 128, "pattern_count": count,
                "min_pattern_len": 1, "max_pattern_len": 16, "alphabet": [0, 1, 2, 3]
            }))
            .unwrap(),
        )
        .unwrap();
        let data = generate(&read_json(&config)).unwrap();
        let patterns = Patterns::new(data.patterns).unwrap();
        let expected = Naive::new(patterns.clone()).find_all(&data.input);
        for (name, strategy) in [
            ("balanced", Strategy::Balanced),
            ("length-sorted", Strategy::LengthSorted),
            ("dynamic-programming", Strategy::DynamicProgramming),
        ] {
            let output_path = directory.path().join(format!("{name}-{count}.json"));
            let output = run(
                "run",
                &config,
                &output_path,
                &[
                    "--algorithm",
                    "fdr-sequential",
                    "--bucket-count",
                    "2",
                    "--filter-length",
                    "8",
                    "--grouping-strategy",
                    name,
                    "--alpha",
                    "1.2",
                    "--beta",
                    "2",
                ],
            );
            assert_success(&output);
            let result: serde_json::Value = read_json(&output_path);
            let filter = FdrFilter::new(
                patterns.clone(),
                NonZeroUsize::new(2).unwrap(),
                NonZeroUsize::new(8).unwrap(),
                strategy,
                Coefficients::new(1.2, 2.0).unwrap(),
            );
            assert_eq!(result["matches"], serde_json::to_value(&expected).unwrap());
            assert_eq!(result["state_width_bits"], filter.state_width());
            assert_eq!(result["state_words"], filter.word_count());
            let saved = &result["fdr_filter"];
            assert_eq!(saved["requested_bucket_count"], 2);
            assert_eq!(saved["filter_length"], 8);
            assert_eq!(saved["grouping_strategy"], name);
            assert_eq!(saved["coefficients"]["alpha"], 1.2);
            assert_eq!(saved["coefficients"]["beta"], 2.0);
            assert_eq!(
                saved["buckets"],
                serde_json::to_value(filter.buckets()).unwrap()
            );
            assert_eq!(
                saved["effective_lengths"],
                serde_json::to_value(filter.effective_lengths()).unwrap()
            );
            assert_eq!(saved["mask_table_bytes"], filter.mask_table_bytes());
            let stats = filter.statistics(&data.input);
            assert_eq!(saved["candidate_count"], stats.candidate_count);
            assert_eq!(saved["full_comparisons"], stats.full_comparisons);
            assert_eq!(
                saved["false_positive_candidates"],
                stats.false_positive_candidates
            );
            assert!(result["construction_ns"].is_number());
        }
    }
    for (flag, value) in [("--alpha", "0"), ("--alpha", "NaN"), ("--beta", "inf")] {
        let result = directory.path().join("invalid-fdr.json");
        let output = Command::new(env!("CARGO_BIN_EXE_simple-fdr"))
            .args(["run", "unused"])
            .arg(&result)
            .args(["--algorithm", "fdr-sequential", flag, value])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("alpha must be finite"));
        assert!(!result.exists());
    }
}

#[test]
fn batched_fdr_records_sizes_and_matches_sequential() {
    let directory = tempfile::tempdir().unwrap();
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("experiments/small.json");
    let run = |algorithm: &str, size: &str| {
        let path = directory.path().join(format!("{algorithm}-{size}.json"));
        let output = run(
            "run",
            &config,
            &path,
            &[
                "--algorithm",
                algorithm,
                "--batch-size",
                size,
                "--bucket-count",
                "8",
                "--filter-length",
                "8",
            ],
        );
        (output, path)
    };
    let (output, path) = run("fdr-sequential", "8");
    assert!(output.status.success());
    let reference: serde_json::Value = read_json(&path);
    assert_eq!(reference["fdr_filter"]["batch_size"], 1);
    for size in ["1", "2", "4", "8"] {
        let (output, path) = run("fdr-batched", size);
        assert_success(&output);
        let result: serde_json::Value = read_json(&path);
        assert_eq!(result["algorithm"], "fdr-batched");
        assert_eq!(
            result["fdr_filter"]["batch_size"],
            size.parse::<usize>().unwrap()
        );
        assert_eq!(result["matches"], reference["matches"]);
        for key in [
            "candidate_count",
            "full_comparisons",
            "false_positive_candidates",
            "buckets",
            "effective_lengths",
            "mask_table_bytes",
        ] {
            assert_eq!(result["fdr_filter"][key], reference["fdr_filter"][key]);
        }
    }
    for size in ["0", "3", "16"] {
        let (output, path) = run("fdr-batched", size);
        assert!(!output.status.success());
        assert!(!path.exists());
    }
}

#[test]
fn super_character_records_bits_table_size_and_reference_matches() {
    let directory = tempfile::tempdir().unwrap();
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("experiments/small.json");
    let data = generate(&read_json(&config)).unwrap();
    let expected = Naive::new(Patterns::new(data.patterns).unwrap()).find_all(&data.input);
    for algorithm in ["fdr-sequential", "fdr-batched"] {
        for bits in ["0", "4", "8", "1", "9"] {
            let path = directory.path().join(format!("{algorithm}-{bits}.json"));
            let output = run(
                "run",
                &config,
                &path,
                &[
                    "--algorithm",
                    algorithm,
                    "--super-character-bits",
                    bits,
                    "--bucket-count",
                    "2",
                    "--filter-length",
                    "8",
                ],
            );
            if bits == "1" || bits == "9" {
                assert_eq!(output.status.code(), Some(2));
                assert!(!path.exists());
                continue;
            }
            assert_success(&output);
            let result: serde_json::Value = read_json(&path);
            let bits: usize = bits.parse().unwrap();
            assert_eq!(result["fdr_filter"]["super_character_bits"], bits);
            assert_eq!(result["matches"], serde_json::to_value(&expected).unwrap());
            assert_eq!(
                result["fdr_filter"]["mask_table_bytes"],
                (256 + if bits == 0 { 0 } else { 256 << bits }) * 8
            );
        }
    }
}

#[test]
fn hash_verifier_records_method_payload_lookups_and_reference_matches() {
    let directory = tempfile::tempdir().unwrap();
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("experiments/abcdqrst-2-6.json");
    let data = generate(&read_json(&config)).unwrap();
    let expected = Naive::new(Patterns::new(data.patterns).unwrap()).find_all(&data.input);
    for algorithm in ["fdr-sequential", "fdr-batched"] {
        for method in ["exhaustive", "hash", "invalid"] {
            let path = directory.path().join(format!("{algorithm}-{method}.json"));
            let output = run(
                "run",
                &config,
                &path,
                &[
                    "--algorithm",
                    algorithm,
                    "--verification",
                    method,
                    "--super-character-bits",
                    "4",
                    "--bucket-count",
                    "8",
                    "--filter-length",
                    "8",
                ],
            );
            if method == "invalid" {
                assert_eq!(output.status.code(), Some(2));
                assert!(!path.exists());
                continue;
            }
            assert_success(&output);
            let result: serde_json::Value = read_json(&path);
            assert_eq!(result["matches"], serde_json::to_value(&expected).unwrap());
            let saved = &result["fdr_filter"];
            assert_eq!(saved["verification"], method);
            if method == "hash" {
                assert_eq!(saved["hash_lookups"], saved["candidate_count"]);
                assert!(saved["verification_table_bytes"].as_u64().unwrap() > 0);
            } else {
                assert_eq!(saved["hash_lookups"], 0);
                assert_eq!(saved["verification_table_bytes"], 0);
            }
        }
    }
}

#[test]
fn simd_backend_is_recorded_and_unsupported_backend_fails() {
    use simple_fdr::simd::Backend;
    let directory = tempfile::tempdir().unwrap();
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("experiments/abcdqrst-2-6.json");
    let mut baseline = None;
    for (name, backend) in [
        ("integer", Backend::Integer),
        ("neon", Backend::Neon),
        ("sse2", Backend::Sse2),
    ] {
        let path = directory.path().join(format!("{name}.json"));
        let output = run(
            "run",
            &config,
            &path,
            &[
                "--algorithm",
                "fdr-batched",
                "--backend",
                name,
                "--bucket-count",
                "8",
                "--filter-length",
                "8",
                "--verification",
                "hash",
            ],
        );
        if backend.is_supported() {
            assert_success(&output);
            let result: serde_json::Value = read_json(&path);
            assert_eq!(result["fdr_filter"]["backend"], name);
            if let Some(expected) = &baseline {
                assert_eq!(&result["matches"], expected);
            } else {
                baseline = Some(result["matches"].clone());
            }
        } else {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("unavailable"));
            assert!(!path.exists());
        }
    }
}
