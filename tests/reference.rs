use simple_fdr::{Match, Patterns, naive::Naive};

#[test]
fn preserves_overlaps_containment_and_duplicate_patterns() {
    let patterns = Patterns::new(vec![
        b"aa".to_vec(),
        b"a".to_vec(),
        b"aa".to_vec(),
        b"aaaa".to_vec(),
    ])
    .unwrap();
    let matcher = Naive::new(patterns);
    assert_eq!(
        matcher.find_all(b"aaa"),
        vec![
            Match {
                pattern_id: 0,
                start: 0,
                end: 2
            },
            Match {
                pattern_id: 0,
                start: 1,
                end: 3
            },
            Match {
                pattern_id: 1,
                start: 0,
                end: 1
            },
            Match {
                pattern_id: 1,
                start: 1,
                end: 2
            },
            Match {
                pattern_id: 1,
                start: 2,
                end: 3
            },
            Match {
                pattern_id: 2,
                start: 0,
                end: 2
            },
            Match {
                pattern_id: 2,
                start: 1,
                end: 3
            },
        ]
    );
}

#[test]
fn handles_empty_input_and_empty_pattern_set() {
    let matcher = Naive::new(Patterns::new(vec![b"abc".to_vec()]).unwrap());
    assert!(matcher.find_all(b"").is_empty());
    assert!(matcher.find_all(b"ab").is_empty());
    let matcher = Naive::new(Patterns::new(vec![]).unwrap());
    assert!(matcher.find_all(b"abc").is_empty());
    assert!(matcher.find_all(b"").is_empty());
}

#[test]
fn rejects_empty_patterns_without_changing_their_ids() {
    let error = Patterns::new(vec![b"a".to_vec(), vec![]]).unwrap_err();
    assert_eq!(error.pattern_id, 1);
}

#[test]
fn supports_every_byte_including_zero() {
    let input: Vec<u8> = (0..=255).collect();
    let patterns = Patterns::new(input.iter().map(|&b| vec![b]).collect()).unwrap();
    let matches = Naive::new(patterns).find_all(&input);
    assert_eq!(matches.len(), 256);
    for (id, m) in matches.iter().enumerate() {
        assert_eq!(
            *m,
            Match {
                pattern_id: id,
                start: id,
                end: id + 1
            }
        );
    }
}

#[test]
fn reports_input_start_and_end() {
    let patterns = Patterns::new(vec![b"abc".to_vec(), b"xyz".to_vec()]).unwrap();
    assert_eq!(
        Naive::new(patterns).find_all(b"abcxyz"),
        vec![
            Match {
                pattern_id: 0,
                start: 0,
                end: 3
            },
            Match {
                pattern_id: 1,
                start: 3,
                end: 6
            },
        ]
    );
}
