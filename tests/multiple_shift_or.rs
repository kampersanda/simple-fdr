use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use simple_fdr::{Match, Patterns, multiple_shift_or::MultipleShiftOr, naive::Naive};

fn compare(patterns: Vec<Vec<u8>>, input: &[u8]) {
    let patterns = Patterns::new(patterns).unwrap();
    let expected = Naive::new(patterns.clone()).find_all(input);
    let matcher = MultipleShiftOr::new(patterns);
    let mut found = matcher.find_all(input);
    let mut scanned = Vec::new();
    matcher.scan(input, |m| scanned.push(m));
    assert_eq!(found, scanned);
    assert!(
        found
            .windows(2)
            .all(|pair| { (pair[0].end, pair[0].pattern_id) <= (pair[1].end, pair[1].pattern_id) })
    );
    found.sort_unstable();
    assert_eq!(found, expected);
    assert_eq!(matcher.find_all(input), scanned); // Searches reset state.
}

#[test]
fn article_layout_masks_dummies_and_acceptance() {
    let matcher = MultipleShiftOr::new(
        Patterns::new(vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()]).unwrap(),
    );
    assert_eq!(matcher.state_width(), 15);
    assert_eq!(matcher.word_count(), 1);
    assert_eq!(matcher.initial_state(), [0b111_111_111_111_000]);
    assert_eq!(matcher.mask(b'a'), [0b111_111_111_100_000]);
    assert_eq!(matcher.mask(b'b'), [0b111_111_110_011_000]);
    assert_eq!(matcher.mask(b'c'), [0b111_110_011_111_000]);
    assert_eq!(matcher.mask(b'd'), [0b110_111_101_111_000]);
    assert_eq!(matcher.mask(b'x'), matcher.initial_state());
    assert_eq!(
        (0..3)
            .map(|id| matcher.accepting_bit(id))
            .collect::<Vec<_>>(),
        [12, 7, 8]
    );
    let trace = matcher.trace(b"abcd");
    assert_eq!(
        trace.iter().map(|s| s.state[0]).collect::<Vec<_>>(),
        [
            0b111_111_111_100_000,
            0b111_111_110_011_000,
            0b111_110_011_111_000,
            0b110_111_111_111_000,
        ]
    );
    assert_eq!(
        trace[2].matches,
        [Match {
            pattern_id: 2,
            start: 1,
            end: 3
        }]
    );
    assert_eq!(
        trace[3].matches,
        [Match {
            pattern_id: 0,
            start: 0,
            end: 4
        }]
    );
    for step in &trace {
        assert_eq!(step.mask, matcher.mask(step.byte));
        // Dummy bits for the two short patterns remain inactive.
        assert_eq!(step.state[0] & 0b110_110_000_000_000, 0b110_110_000_000_000);
    }
    compare(
        vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()],
        b"abcdadbc",
    );
}

#[test]
fn word_boundaries_and_shifts_of_whole_words() {
    for count in [1, 2, 3, 31, 32, 33, 63, 64, 65, 100, 127, 128, 129] {
        for length in [1, 2, 16, 63, 64, 65, 129] {
            // Overlapping matches exercise carries and every pattern lane.
            compare(vec![vec![0; length]; count], &vec![0; length + 2]);
        }
    }
    let matcher = MultipleShiftOr::new(Patterns::new(vec![vec![0; 16]; 100]).unwrap());
    assert_eq!(matcher.state_width(), 1700);
    assert_eq!(matcher.word_count(), 27);
}

#[test]
fn empty_sets_inputs_duplicates_containment_and_all_bytes() {
    compare(vec![], b"abc");
    compare(vec![b"a".to_vec()], b"");
    compare(
        vec![
            b"aa".to_vec(),
            b"a".to_vec(),
            b"aa".to_vec(),
            b"aaaa".to_vec(),
        ],
        b"aaa",
    );
    let bytes: Vec<u8> = (0..=255).collect();
    compare(bytes.iter().map(|&byte| vec![byte]).collect(), &bytes);
    compare(vec![bytes.clone()], &bytes);
    let empty = MultipleShiftOr::new(Patterns::new(vec![]).unwrap());
    assert_eq!(empty.state_width(), 0);
    assert_eq!(empty.word_count(), 0);
    assert!(empty.trace(b"abc").is_empty());
}

#[test]
fn fixed_seed_mixed_lengths_match_reference() {
    let mut rng = ChaCha8Rng::seed_from_u64(29);
    for alphabet in [4u16, 256] {
        for count in [1, 3, 17, 63, 64, 65, 100, 129] {
            for _ in 0..8 {
                let patterns: Vec<Vec<u8>> = (0..count)
                    .map(|_| {
                        let length = rng.random_range(1..=90);
                        (0..length)
                            .map(|_| rng.random_range(0..alphabet) as u8)
                            .collect()
                    })
                    .collect();
                let mut input: Vec<u8> = (0..240)
                    .map(|_| rng.random_range(0..alphabet) as u8)
                    .collect();
                for id in [0, count / 2, count - 1] {
                    let pattern = &patterns[id];
                    let start = rng.random_range(0..=input.len() - pattern.len());
                    input[start..start + pattern.len()].copy_from_slice(pattern);
                }
                compare(patterns, &input);
            }
        }
    }
}
