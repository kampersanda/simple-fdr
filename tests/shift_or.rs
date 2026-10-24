use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use simple_fdr::{
    Match, Patterns,
    naive::Naive,
    shift_or::{MAX_PATTERN_LEN, ShiftOr},
};

fn compare(pattern: &[u8], input: &[u8]) {
    let expected = Naive::new(Patterns::new(vec![pattern.to_vec()]).unwrap()).find_all(input);
    let matcher = ShiftOr::new(pattern).unwrap();
    assert_eq!(
        matcher.find_all(input),
        expected,
        "pattern={pattern:?}, input={input:?}"
    );
    let mut scanned = Vec::new();
    matcher.scan(input, |m| scanned.push(m));
    assert_eq!(scanned, expected);
}

#[test]
fn reproduces_article_masks_and_states() {
    let matcher = ShiftOr::new(b"acbc").unwrap();
    assert_eq!(matcher.state_width(), 5);
    assert_eq!(matcher.initial_state(), 0b11110);
    assert_eq!(matcher.mask(b'a'), 0b11100);
    assert_eq!(matcher.mask(b'b'), 0b10110);
    assert_eq!(matcher.mask(b'c'), 0b01010);
    assert_eq!(matcher.mask(b'x'), 0b11110);
    let trace = matcher.trace(b"aacbc");
    assert_eq!(
        trace.iter().map(|s| s.state).collect::<Vec<_>>(),
        [0b11100, 0b11100, 0b11010, 0b10110, 0b01110]
    );
    for (offset, step) in trace.iter().enumerate() {
        assert_eq!(step.end, offset + 1);
        assert_eq!(step.byte, b"aacbc"[offset]);
        assert_eq!(step.mask, matcher.mask(step.byte));
        assert_eq!(step.matched, offset == 4);
    }
    assert_eq!(
        matcher.find_all(b"aacbc"),
        [Match {
            pattern_id: 0,
            start: 1,
            end: 5
        }]
    );
    let classes = vec![b"ac".to_vec(), b"c".to_vec(), b"b".to_vec(), b"c".to_vec()];
    let matcher = ShiftOr::from_classes(&classes).unwrap();
    assert_eq!(matcher.mask(b'a'), 0b11100);
    assert_eq!(matcher.mask(b'b'), 0b10110);
    assert_eq!(matcher.mask(b'c'), 0b01000);
    assert_eq!(
        matcher.find_all(b"acbccbc"),
        [
            Match {
                pattern_id: 0,
                start: 0,
                end: 4
            },
            Match {
                pattern_id: 0,
                start: 3,
                end: 7
            }
        ]
    );
}

#[test]
fn handles_boundaries_overlaps_and_every_byte() {
    for length in 1..=MAX_PATTERN_LEN {
        let pattern = vec![0; length];
        for input_len in [0, length - 1, length, length + 2] {
            compare(&pattern, &vec![0; input_len]);
        }
    }
    for byte in 0..=255 {
        compare(&[byte], &[byte, byte.wrapping_add(1), byte]);
    }
    compare(b"abc", b"abcxabc");
    let matcher = ShiftOr::new(&[0; MAX_PATTERN_LEN]).unwrap();
    assert_eq!(matcher.state_width(), 64);
    assert_eq!(matcher.initial_state(), u64::MAX - 1);
    assert_eq!(matcher.mask(0), 0);
    assert_eq!(matcher.mask(1), u64::MAX - 1);
}

#[test]
fn rejects_lengths_outside_one_word() {
    for length in [0, 64, 65] {
        assert_eq!(ShiftOr::new(&vec![0; length]).unwrap_err().length, length);
        assert_eq!(
            ShiftOr::from_classes(&vec![vec![0]; length])
                .unwrap_err()
                .length,
            length
        );
    }
}

#[test]
fn fixed_seed_literals_match_reference() {
    let mut rng = ChaCha8Rng::seed_from_u64(17);
    for alphabet_size in [4u16, 256] {
        for length in 1..=MAX_PATTERN_LEN {
            for _ in 0..12 {
                let pattern: Vec<u8> = (0..length)
                    .map(|_| rng.random_range(0..alphabet_size) as u8)
                    .collect();
                let mut input: Vec<u8> = (0..160)
                    .map(|_| rng.random_range(0..alphabet_size) as u8)
                    .collect();
                let start = rng.random_range(0..=input.len() - length);
                input[start..start + length].copy_from_slice(&pattern);
                compare(&pattern, &input);
            }
        }
    }
}

#[test]
fn fixed_seed_classes_match_direct_window_comparison() {
    let mut rng = ChaCha8Rng::seed_from_u64(23);
    for length in 1..=MAX_PATTERN_LEN {
        for iteration in 0..12 {
            let classes: Vec<Vec<u8>> = (0..length)
                .map(|_| {
                    (0..rng.random_range((iteration % 2)..=8))
                        .map(|_| rng.random_range(0..8))
                        .collect()
                })
                .collect();
            let mut input: Vec<u8> = (0..160).map(|_| rng.random_range(0..8)).collect();
            // Plant an accepted window whenever every position has an allowed byte.
            if classes.iter().all(|class| !class.is_empty()) {
                for (byte, class) in input.iter_mut().zip(&classes) {
                    *byte = class[0];
                }
            }
            let expected: Vec<Match> = input
                .windows(length)
                .enumerate()
                .filter(|(_, window)| {
                    window
                        .iter()
                        .zip(&classes)
                        .all(|(byte, class)| class.contains(byte))
                })
                .map(|(start, _)| Match {
                    pattern_id: 0,
                    start,
                    end: start + length,
                })
                .collect();
            let matcher = ShiftOr::from_classes(&classes).unwrap();
            assert_eq!(matcher.find_all(&input), expected);
            for byte in 0..=255 {
                assert_eq!(matcher.mask(byte) & 1, 0);
            }
        }
    }
    let any = ShiftOr::from_classes(&[(0..=255).collect()]).unwrap();
    assert_eq!(any.find_all(&(0..=255).collect::<Vec<u8>>()).len(), 256);
    assert!(
        ShiftOr::from_classes(&[vec![]])
            .unwrap()
            .find_all(b"abc")
            .is_empty()
    );
}
