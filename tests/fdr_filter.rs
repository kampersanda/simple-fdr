use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use simple_fdr::{
    Patterns,
    fdr_filter::FdrFilter,
    filter::{Candidate, SearchStats},
    grouping::{Coefficients, Strategy, partition},
    naive::Naive,
};
use std::num::NonZeroUsize;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}
const STRATEGIES: [Strategy; 3] = [
    Strategy::Balanced,
    Strategy::LengthSorted,
    Strategy::DynamicProgramming,
];

fn compare(bytes: Vec<Vec<u8>>, input: &[u8], q: usize, length: usize, strategy: Strategy) {
    let patterns = Patterns::new(bytes).unwrap();
    let expected = Naive::new(patterns.clone()).find_all(input);
    let filter = FdrFilter::new(
        patterns.clone(),
        nz(q),
        nz(length),
        strategy,
        Coefficients::default(),
    );
    let mut found = filter.find_all(input);
    let mut scanned = Vec::new();
    filter.scan(input, |m| scanned.push(m));
    assert_eq!(found, scanned);
    assert_eq!(found, filter.find_all(input));
    found.sort_unstable();
    assert_eq!(found, expected);
    // Independent oracle: compare the input suffix against the union of bytes
    // at each reverse position, without state masks or bit operations.
    let mut candidates = Vec::new();
    for end in 1..=input.len() {
        for (b, bucket) in filter.buckets().iter().enumerate() {
            let length = filter.effective_lengths()[b];
            if end >= length
                && (0..length).all(|d| {
                    bucket.iter().any(|&id| {
                        let pattern = &patterns.as_slice()[id];
                        input[end - 1 - d] == pattern[pattern.len() - 1 - d]
                    })
                })
            {
                candidates.push(Candidate { bucket_id: b, end });
            }
        }
    }
    let mut actual = Vec::new();
    filter.scan_candidates(input, |c| actual.push(c));
    assert_eq!(actual, candidates);
    assert_eq!(filter.statistics(input).candidate_count, candidates.len());
}

#[test]
fn reverse_layout_padding_and_initialization() {
    let filter = FdrFilter::new(
        Patterns::new(vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()]).unwrap(),
        nz(2),
        nz(8),
        Strategy::Balanced,
        Coefficients::default(),
    );
    assert_eq!(filter.buckets(), [vec![0, 1], vec![2]]);
    assert_eq!(filter.effective_lengths(), [2, 2]);
    assert_eq!(filter.state_width(), 4); // No explicit start-state row.
    assert_eq!(filter.initial_state(), [0b0011]);
    assert_eq!(filter.mask(b'd'), [0b1110]);
    assert_eq!(filter.mask(b'a'), [0b1011]);
    assert_eq!(filter.mask_table_bytes(), 2048);
    // A separate length-one bucket has zero padding in row 1 for every byte.
    let padded = FdrFilter::new(
        Patterns::new(vec![b"a".to_vec(), b"bc".to_vec()]).unwrap(),
        nz(2),
        nz(8),
        Strategy::Balanced,
        Coefficients::default(),
    );
    assert_eq!(padded.effective_lengths(), [1, 2]);
    assert_eq!(padded.initial_state(), [0b0010]);
    assert!((0..=255).all(|byte| padded.mask(byte)[0] & 0b0100 == 0));
    assert!(padded.find_all(b"c").is_empty());
    assert_eq!(padded.find_all(b"abc").len(), 2);
}

#[test]
fn empty_mixed_lengths_all_bytes_and_word_boundaries() {
    for strategy in STRATEGIES {
        for q in [1, 2, 3, 63, 64, 65, 127, 128, 129, 300] {
            for length in [1, 2, 65, 300] {
                compare(vec![], b"abc", q, length, strategy);
                compare(vec![b"ab".to_vec()], b"", q, length, strategy);
                compare(
                    vec![
                        b"a".to_vec(),
                        b"aa".to_vec(),
                        b"aa".to_vec(),
                        b"aaaa".to_vec(),
                        b"long".to_vec(),
                    ],
                    b"aaaa",
                    q,
                    length,
                    strategy,
                );
            }
        }
        let bytes: Vec<u8> = (0..=255).collect();
        compare(
            bytes.iter().map(|&b| vec![b]).collect(),
            &bytes,
            65,
            8,
            strategy,
        );
        compare(vec![bytes.clone()], &bytes, 1, 300, strategy);
        for q in [1, 2, 3, 63, 64, 65, 127, 128, 129] {
            let patterns = (0..q * 2).map(|id| vec![0; 63 + id % 3]).collect();
            compare(patterns, &[0; 68], q, 65, strategy);
        }
    }
    let empty = FdrFilter::new(
        Patterns::new(vec![]).unwrap(),
        nz(8),
        nz(8),
        Strategy::Balanced,
        Coefficients::default(),
    );
    assert_eq!(empty.state_width(), 0);
    assert_eq!(empty.mask_table_bytes(), 0);
    assert_eq!(empty.statistics(b"abc"), SearchStats::default());
}

#[test]
fn random_differential_matches_and_candidate_sets() {
    let mut rng = ChaCha8Rng::seed_from_u64(41);
    for alphabet in [4u16, 256] {
        for count in [1, 3, 17, 65] {
            for _ in 0..4 {
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
                for strategy in STRATEGIES {
                    for q in [1, 2, count] {
                        for length in [1, 4, 65] {
                            compare(patterns.clone(), &input, q, length, strategy);
                        }
                    }
                }
            }
        }
    }
}

fn cost(patterns: &Patterns, buckets: &[Vec<usize>], c: Coefficients) -> f64 {
    buckets
        .iter()
        .map(|bucket| {
            (bucket.len() as f64).powf(c.alpha())
                / (bucket
                    .iter()
                    .map(|&id| patterns.as_slice()[id].len())
                    .min()
                    .unwrap() as f64)
                    .powf(c.beta())
        })
        .sum()
}

fn exhaustive(lengths: &[usize], q: usize, c: Coefficients) -> f64 {
    if q == 1 {
        return (lengths.len() as f64).powf(c.alpha()) / (lengths[0] as f64).powf(c.beta());
    }
    (1..=lengths.len() - q + 1)
        .map(|split| {
            (split as f64).powf(c.alpha()) / (lengths[0] as f64).powf(c.beta())
                + exhaustive(&lengths[split..], q - 1, c)
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn dp_matches_exhaustive_contiguous_partitions_and_preserves_ids() {
    let lengths = [16, 1, 8, 1, 32, 3, 8, 2];
    let patterns = Patterns::new(lengths.iter().map(|&n| vec![0; n]).collect()).unwrap();
    let mut sorted = lengths.to_vec();
    sorted.sort();
    for c in [
        Coefficients::default(),
        Coefficients::new(2.0, 1.0).unwrap(),
        Coefficients::new(1.0, 0.0).unwrap(),
    ] {
        for q in 1..=lengths.len() {
            let buckets = partition(&patterns, nz(q), Strategy::DynamicProgramming, c);
            assert_eq!(buckets.len(), q);
            assert!(buckets.iter().all(|b| !b.is_empty()));
            assert_eq!(buckets.concat(), [1, 3, 7, 5, 2, 6, 0, 4]);
            assert!((cost(&patterns, &buckets, c) - exhaustive(&sorted, q, c)).abs() < 1e-10);
            assert_eq!(
                buckets,
                partition(&patterns, nz(q), Strategy::DynamicProgramming, c)
            );
        }
    }
    assert_eq!(
        partition(
            &patterns,
            nz(3),
            Strategy::Balanced,
            Coefficients::default()
        ),
        [vec![0, 1, 2], vec![3, 4, 5], vec![6, 7]]
    );
    assert_eq!(
        partition(
            &patterns,
            nz(3),
            Strategy::LengthSorted,
            Coefficients::default()
        ),
        [vec![1, 3, 7], vec![5, 2, 6], vec![0, 4]]
    );
}

#[test]
fn invalid_coefficients_are_rejected() {
    for (alpha, beta) in [
        (0.0, 3.0),
        (-1.0, 3.0),
        (f64::NAN, 3.0),
        (f64::INFINITY, 3.0),
        (1.05, -1.0),
        (1.05, f64::NAN),
        (1.05, f64::INFINITY),
    ] {
        assert!(Coefficients::new(alpha, beta).is_err());
    }
}

#[test]
fn statistics_count_events_separately_from_matches_and_skip_short_prefixes() {
    let filter = FdrFilter::new(
        Patterns::new(vec![b"ab".to_vec(), b"cd".to_vec()]).unwrap(),
        nz(1),
        nz(8),
        Strategy::Balanced,
        Coefficients::default(),
    );
    assert_eq!(
        filter.statistics(b"ad"),
        SearchStats {
            candidate_count: 1,
            full_comparisons: 2,
            false_positive_candidates: 1,
        }
    );
    let duplicates = FdrFilter::new(
        Patterns::new(vec![b"a".to_vec(), b"aa".to_vec(), b"aa".to_vec()]).unwrap(),
        nz(1),
        nz(8),
        Strategy::Balanced,
        Coefficients::default(),
    );
    assert_eq!(duplicates.find_all(b"aa").len(), 4);
    assert_eq!(
        duplicates.statistics(b"aa"),
        SearchStats {
            candidate_count: 2,
            full_comparisons: 4,
            false_positive_candidates: 0,
        }
    );
    let truncated = FdrFilter::new(
        Patterns::new(vec![b"abcd".to_vec()]).unwrap(),
        nz(1),
        nz(2),
        Strategy::Balanced,
        Coefficients::default(),
    );
    assert_eq!(
        truncated.statistics(b"cd"),
        SearchStats {
            candidate_count: 1,
            full_comparisons: 0,
            false_positive_candidates: 1,
        }
    );
}
