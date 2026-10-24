use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use simple_fdr::{
    Patterns,
    fdr_filter::{BatchSize, FdrFilter, SuperCharacterBits},
    filter::Candidate,
    grouping::{Coefficients, Strategy},
    naive::Naive,
};
use std::num::NonZeroUsize;

const BITS: [SuperCharacterBits; 3] = [
    SuperCharacterBits::Zero,
    SuperCharacterBits::Four,
    SuperCharacterBits::Eight,
];
const BATCHES: [BatchSize; 4] = [
    BatchSize::One,
    BatchSize::Two,
    BatchSize::Four,
    BatchSize::Eight,
];

fn filter(patterns: Patterns, q: usize, length: usize, strategy: Strategy) -> FdrFilter {
    FdrFilter::new(
        patterns,
        NonZeroUsize::new(q).unwrap(),
        NonZeroUsize::new(length).unwrap(),
        strategy,
        Coefficients::default(),
    )
}

fn compare(bytes: Vec<Vec<u8>>, input: &[u8], q: usize, length: usize, strategy: Strategy) {
    let patterns = Patterns::new(bytes).unwrap();
    let expected = Naive::new(patterns.clone()).find_all(input);
    let mut previous = Vec::new();
    for bits in BITS {
        let mut matcher = filter(patterns.clone(), q, length, strategy).with_super_character(bits);
        // Independent oracle: position-wise union of correlated byte pairs.
        let mut candidates = Vec::new();
        let low_mask = (1usize << bits.get()) - 1;
        for end in 1..=input.len() {
            for (b, bucket) in matcher.buckets().iter().enumerate() {
                let length = matcher.effective_lengths()[b];
                if end >= length
                    && (0..length).all(|d| {
                        let index = end - 1 - d;
                        bucket.iter().any(|&id| {
                            let pattern = &patterns.as_slice()[id];
                            let position = pattern.len() - 1 - d;
                            input[index] == pattern[position]
                                && (d == 0
                                    || index + 1 == input.len()
                                    || (input[index + 1] as usize & low_mask)
                                        == (pattern[position + 1] as usize & low_mask))
                        })
                    })
                {
                    candidates.push(Candidate { bucket_id: b, end });
                }
            }
        }
        if bits != SuperCharacterBits::Zero {
            assert!(candidates.iter().all(|c| previous.contains(c)));
        }
        previous = candidates.clone();
        let statistics = matcher.statistics(input);
        for batch in BATCHES {
            matcher = matcher.with_batch_size(batch);
            let mut actual = Vec::new();
            matcher.scan_candidates(input, |c| actual.push(c));
            assert_eq!(actual, candidates, "bits={bits:?}, batch={batch:?}");
            let mut found = matcher.find_all(input);
            let mut scanned = Vec::new();
            matcher.scan(input, |m| scanned.push(m));
            assert_eq!(found, scanned);
            found.sort_unstable();
            assert_eq!(found, expected, "bits={bits:?}, batch={batch:?}");
            assert_eq!(matcher.statistics(input), statistics);
            assert_eq!(statistics.candidate_count, candidates.len());
        }
    }
}

#[test]
fn grouping_false_positives_and_low_bit_collisions() {
    for bits in BITS {
        let matcher = filter(
            Patterns::new(vec![b"ab".to_vec(), b"cd".to_vec()]).unwrap(),
            1,
            8,
            Strategy::Balanced,
        )
        .with_super_character(bits);
        assert_eq!(
            matcher.statistics(b"adcb").candidate_count,
            if bits == SuperCharacterBits::Zero {
                2
            } else {
                0
            }
        );
        assert_eq!(
            matcher.mask_table_bytes(),
            (256 + if bits == SuperCharacterBits::Zero {
                0
            } else {
                256 << bits.get()
            }) * 8
        );
        assert_eq!(matcher.super_character_bits(), bits);
        for next in 0..=255 {
            // Terminal 'b' accepts every next value; padding remains zero.
            assert_eq!(matcher.super_character_mask(b'b', Some(next))[0] & 1, 0);
        }
        assert_eq!(matcher.super_character_mask(b'b', None), matcher.mask(b'b'));
    }
    // 0x02 and 0x12 collide with four added bits, but not eight.
    let patterns = Patterns::new(vec![vec![b'a', 0x02], vec![b'c', 0x12]]).unwrap();
    let four = filter(patterns.clone(), 1, 8, Strategy::Balanced)
        .with_super_character(SuperCharacterBits::Four);
    let eight =
        filter(patterns, 1, 8, Strategy::Balanced).with_super_character(SuperCharacterBits::Eight);
    assert_eq!(four.statistics(&[b'a', 0x12]).candidate_count, 1);
    assert_eq!(eight.statistics(&[b'a', 0x12]).candidate_count, 0);
}

#[test]
fn terminal_wildcard_eof_and_every_block_position() {
    for start in 0..16 {
        for tail in 0..9 {
            let mut input = vec![b'x'; start];
            input.extend_from_slice(b"ab");
            input.extend(vec![b'x'; tail]);
            compare(
                vec![b"ab".to_vec(), b"cd".to_vec(), b"ab".to_vec()],
                &input,
                1,
                8,
                Strategy::Balanced,
            );
        }
    }
    for bytes in [vec![], vec![b"a".to_vec()], vec![b"ab".to_vec()]] {
        compare(bytes, b"", 8, 8, Strategy::Balanced);
    }
    compare(
        vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()],
        b"abcdadbc",
        2,
        8,
        Strategy::Balanced,
    );
    compare(
        vec![
            b"a".to_vec(),
            b"aa".to_vec(),
            b"aa".to_vec(),
            b"aaaa".to_vec(),
        ],
        b"aaaa",
        1,
        8,
        Strategy::Balanced,
    );
}

#[test]
fn all_bytes_truncation_and_word_boundaries() {
    let bytes: Vec<u8> = (0..=255).collect();
    compare(vec![bytes.clone()], &bytes, 1, 65, Strategy::Balanced);
    compare(
        bytes.iter().map(|&b| vec![b]).collect(),
        &bytes,
        65,
        8,
        Strategy::Balanced,
    );
    for q in [1, 63, 64, 65] {
        let patterns = (0..q * 2)
            .map(|id| vec![(id % 3) as u8; 7 + id % 3])
            .collect();
        compare(patterns, &[0; 19], q, 9, Strategy::Balanced);
    }
    let empty = filter(Patterns::new(vec![]).unwrap(), 8, 8, Strategy::Balanced)
        .with_super_character(SuperCharacterBits::Eight);
    assert_eq!(empty.mask_table_bytes(), 0);
    assert_eq!(empty.statistics(b"abc").candidate_count, 0);
}

#[test]
fn random_differential_pairs_and_matches() {
    let mut rng = ChaCha8Rng::seed_from_u64(67);
    for alphabet in [4u16, 256] {
        for count in [1, 7, 65] {
            let patterns: Vec<Vec<u8>> = (0..count)
                .map(|_| {
                    let length = rng.random_range(1..=20);
                    (0..length)
                        .map(|_| rng.random_range(0..alphabet) as u8)
                        .collect()
                })
                .collect();
            let mut input: Vec<u8> = (0..97)
                .map(|_| rng.random_range(0..alphabet) as u8)
                .collect();
            for id in [0, count / 2, count - 1] {
                let pattern = &patterns[id];
                let start = rng.random_range(0..=input.len() - pattern.len());
                input[start..start + pattern.len()].copy_from_slice(pattern);
            }
            for strategy in [
                Strategy::Balanced,
                Strategy::LengthSorted,
                Strategy::DynamicProgramming,
            ] {
                for (q, length) in [(1, 4), (3, 8), (count, 9)] {
                    compare(patterns.clone(), &input, q, length, strategy);
                }
            }
        }
    }
}
