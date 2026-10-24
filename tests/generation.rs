use simple_fdr::{
    Match, Patterns,
    generate::{Config, Insertion, WordsConfig, generate, sample_words},
    naive::Naive,
};

fn config() -> Config {
    Config {
        seed: 42,
        input_len: 12,
        pattern_count: 2,
        min_pattern_len: 2,
        max_pattern_len: 4,
        alphabet: vec![0, 127, 255],
        insertions: vec![
            Insertion {
                pattern_id: 0,
                start: 0,
            },
            Insertion {
                pattern_id: 1,
                start: 8,
            },
        ],
    }
}

#[test]
fn generator_v2_is_reproducible_and_has_a_stable_fixture() {
    let config = config();
    let first = generate(&config).unwrap();
    assert_eq!(first, generate(&config).unwrap());
    assert_eq!(first.generator_version, 2);
    assert_eq!(first.patterns, vec![vec![255, 127, 127, 0], vec![0, 255]]);
    assert_eq!(
        first.input,
        vec![255, 127, 127, 0, 255, 127, 255, 0, 0, 255, 127, 0]
    );
    assert_eq!(
        first.planted_matches,
        vec![
            Match {
                pattern_id: 0,
                start: 0,
                end: 4
            },
            Match {
                pattern_id: 1,
                start: 8,
                end: 10
            },
        ]
    );
    let mut different = config.clone();
    different.seed += 1;
    assert_ne!(first.patterns, generate(&different).unwrap().patterns);
}

#[test]
fn every_planted_match_survives_and_json_round_trips() {
    for seed in 0..32 {
        let mut config = config();
        config.seed = seed;
        let data = generate(&config).unwrap();
        assert!(data.patterns.iter().all(|p| (2..=4).contains(&p.len())));
        let matcher = Naive::new(Patterns::new(data.patterns.clone()).unwrap());
        let matches = matcher.find_all(&data.input);
        for planted in &data.planted_matches {
            assert!(matches.contains(planted));
        }
        let json = serde_json::to_vec(&data).unwrap();
        assert_eq!(data, serde_json::from_slice(&json).unwrap());
    }
}

#[test]
fn adjacent_insertions_can_end_at_input_boundary() {
    let mut config = config();
    config.min_pattern_len = 3;
    config.max_pattern_len = 3;
    config.input_len = 6;
    config.insertions[1].start = 3;
    assert_eq!(generate(&config).unwrap().planted_matches[1].end, 6);
}

#[test]
fn rejects_invalid_generation_and_insertion_settings() {
    let original = config();
    let mut invalid = original.clone();
    invalid.min_pattern_len = 0;
    assert!(generate(&invalid).is_err());
    invalid = original.clone();
    invalid.max_pattern_len = 1;
    assert!(generate(&invalid).is_err());
    invalid = original.clone();
    invalid.alphabet.clear();
    assert!(generate(&invalid).is_err());
    invalid = original.clone();
    invalid.insertions[0].pattern_id = 2;
    assert!(generate(&invalid).is_err());
    invalid = original.clone();
    invalid.insertions[1].start = 11;
    assert!(generate(&invalid).is_err());
    invalid = original.clone();
    invalid.insertions[1].start = usize::MAX;
    assert!(generate(&invalid).is_err());
    invalid = original;
    invalid.insertions[1].start = 1;
    assert!(generate(&invalid).is_err());
}

#[test]
fn supports_empty_workloads_and_default_full_byte_alphabet() {
    let config: Config = serde_json::from_str(
        r#"{
        "seed": 0, "input_len": 0, "pattern_count": 0,
        "min_pattern_len": 1, "max_pattern_len": 1
    }"#,
    )
    .unwrap();
    assert_eq!(config.alphabet, (0..=255).collect::<Vec<_>>());
    let data = generate(&config).unwrap();
    assert!(data.input.is_empty());
    assert!(data.patterns.is_empty());
    assert!(data.planted_matches.is_empty());
}

fn words_config(pattern_count: usize) -> WordsConfig {
    WordsConfig {
        seed: 7,
        text: "unused".into(),
        pattern_count,
        min_pattern_len: 2,
        max_pattern_len: 3,
    }
}

const TEXT: &[u8] = b"A cat's hat, the CAT and the dog-cat.\r\nx 42 abcd";

#[test]
fn samples_distinct_words_in_the_length_range_reproducibly() {
    let data = sample_words(&words_config(6), TEXT.to_vec()).unwrap();
    assert_eq!(data.input, TEXT);
    assert!(data.planted_matches.is_empty());
    let mut words = data.patterns.clone();
    words.sort();
    let expected: Vec<&[u8]> = vec![b"CAT", b"and", b"cat", b"dog", b"hat", b"the"];
    assert_eq!(words, expected);
    assert_eq!(data, sample_words(&words_config(6), TEXT.to_vec()).unwrap());

    let fewer = sample_words(&words_config(3), TEXT.to_vec()).unwrap();
    assert_eq!(fewer.patterns.len(), 3);
    assert!(fewer.patterns.iter().all(|p| words.contains(p)));
}

#[test]
fn rejects_invalid_word_settings() {
    assert!(sample_words(&words_config(7), TEXT.to_vec()).is_err());
    let mut invalid = words_config(1);
    invalid.min_pattern_len = 0;
    assert!(sample_words(&invalid, TEXT.to_vec()).is_err());
    invalid.min_pattern_len = 4;
    assert!(sample_words(&invalid, TEXT.to_vec()).is_err());
}
