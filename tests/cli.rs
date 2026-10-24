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
    Command::new(env!("CARGO_BIN_EXE_simple-fdr"))
        .arg(command)
        .arg(config)
        .arg(output)
        .output()
        .unwrap()
}

#[test]
fn saves_reproducible_data_and_complete_experiment_results() {
    let directory = std::env::temp_dir().join(format!("simple-fdr-cli-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("experiments/stage0.json");
    let dataset_path = directory.join("dataset.json");
    let result_path = directory.join("result.json");

    let output = cli("generate", &config, &dataset_path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: Dataset = serde_json::from_slice(&fs::read(&dataset_path).unwrap()).unwrap();
    assert_eq!(data, generate(&data.config).unwrap());

    let output = cli("run", &config, &result_path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value =
        serde_json::from_slice(&fs::read(&result_path).unwrap()).unwrap();
    let expected = Naive::new(Patterns::new(data.patterns).unwrap()).find_all(&data.input);
    assert_eq!(result["format_version"], 1);
    assert_eq!(result["generator_version"], 1);
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
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn invalid_config_fails_without_creating_a_result() {
    let directory = std::env::temp_dir().join(format!("simple-fdr-invalid-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let config = directory.join("config.json");
    let result = directory.join("result.json");
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
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn help_succeeds_and_unknown_commands_fail() {
    assert!(
        Command::new(env!("CARGO_BIN_EXE_simple-fdr"))
            .arg("--help")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        !cli("unknown", Path::new("unused"), Path::new("unused"))
            .status
            .success()
    );
}
