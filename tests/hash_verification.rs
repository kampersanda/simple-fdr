use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use simple_fdr::{
    Patterns,
    fdr_filter::{BatchSize, FdrFilter, SuperCharacterBits, Verification},
    grouping::{Coefficients, Strategy},
    naive::Naive,
};
use std::num::NonZeroUsize;

fn compare(bytes: Vec<Vec<u8>>, input: &[u8], q: usize, length: usize, strategy: Strategy) {
    let patterns = Patterns::new(bytes).unwrap();
    let expected = Naive::new(patterns.clone()).find_all(input);
    for bits in [
        SuperCharacterBits::Zero,
        SuperCharacterBits::Four,
        SuperCharacterBits::Eight,
    ] {
        let mut filter = FdrFilter::new(
            patterns.clone(),
            NonZeroUsize::new(q).unwrap(),
            NonZeroUsize::new(length).unwrap(),
            strategy,
            Coefficients::default(),
        )
        .with_super_character(bits);
        for size in [
            BatchSize::One,
            BatchSize::Two,
            BatchSize::Four,
            BatchSize::Eight,
        ] {
            filter = filter
                .with_batch_size(size)
                .with_verification(Verification::Exhaustive);
            assert_eq!(filter.verification_table_bytes(), 0);
            let exhaustive = filter.find_all(input);
            let stats = filter.statistics(input);
            let mut candidates = Vec::new();
            filter.scan_candidates(input, |c| candidates.push(c));
            filter = filter.with_verification(Verification::Hash);
            assert_eq!(filter.verification(), Verification::Hash);
            let mut found = filter.find_all(input);
            assert_eq!(found, exhaustive); // Original report order and duplicate IDs survive.
            found.sort_unstable();
            assert_eq!(found, expected);
            let mut hash_candidates = Vec::new();
            filter.scan_candidates(input, |c| hash_candidates.push(c));
            assert_eq!(hash_candidates, candidates);
            let hash_stats = filter.statistics(input);
            assert_eq!(hash_stats.candidate_count, stats.candidate_count);
            assert_eq!(
                hash_stats.false_positive_candidates,
                stats.false_positive_candidates
            );
            assert_eq!(hash_stats.hash_lookups, stats.candidate_count);
            assert_eq!(stats.hash_lookups, 0);
            assert!(hash_stats.full_comparisons <= stats.full_comparisons);
        }
    }
}

#[test]
fn mixed_lengths_duplicates_overlaps_empty_and_all_bytes() {
    for input in [b"".as_slice(), b"aaaaabacabxabab"] {
        compare(vec![], input, 8, 8, Strategy::Balanced);
        compare(
            vec![
                b"a".to_vec(),
                b"aa".to_vec(),
                b"aa".to_vec(),
                b"aaaa".to_vec(),
                b"ab".to_vec(),
                b"cab".to_vec(),
                b"longer-than-input-ab".to_vec(),
            ],
            input,
            2,
            8,
            Strategy::Balanced,
        );
    }
    let bytes: Vec<u8> = (0..=255).collect();
    compare(
        bytes.iter().map(|&b| vec![b]).collect(),
        &bytes,
        65,
        8,
        Strategy::Balanced,
    );
    compare(
        vec![bytes.clone(), bytes[1..].to_vec(), bytes.clone()],
        &bytes,
        1,
        65,
        Strategy::Balanced,
    );
    for offset in 0..8 {
        let mut input = vec![0; offset];
        input.extend_from_slice(b"xababcdefab");
        compare(
            vec![
                b"xab".to_vec(),
                b"ab".to_vec(),
                b"abcdefab".to_vec(),
                b"cab".to_vec(),
            ],
            &input,
            1,
            2,
            Strategy::Balanced,
        );
    }
}

#[test]
fn hash_misses_shared_suffixes_and_reduced_comparisons() {
    let patterns = Patterns::new(vec![b"xab".to_vec(), b"yab".to_vec(), b"cd".to_vec()]).unwrap();
    let filter = FdrFilter::new(
        patterns,
        NonZeroUsize::new(1).unwrap(),
        NonZeroUsize::new(2).unwrap(),
        Strategy::Balanced,
        Coefficients::default(),
    );
    let exhaustive = filter.statistics(b"xadxab"); // 'ad' is a filter candidate, but no suffix key.
    let filter = filter.with_verification(Verification::Hash);
    let hashed = filter.statistics(b"xadxab");
    assert_eq!(hashed.candidate_count, 2);
    assert_eq!(hashed.hash_lookups, 2);
    assert_eq!(hashed.full_comparisons, 2);
    assert_eq!(hashed.false_positive_candidates, 1);
    assert_eq!(exhaustive.full_comparisons, 6);
    assert_eq!(
        filter.verification_table_bytes(),
        2 * 8 + 3 * size_of::<usize>()
    );
    assert_eq!(filter.find_all(b"xab").len(), 1);
}

#[test]
fn fixed_seed_differential_all_grouping_strategies() {
    let mut rng = ChaCha8Rng::seed_from_u64(71);
    for alphabet in [4u16, 256] {
        for count in [1, 9, 65] {
            let patterns: Vec<Vec<u8>> = (0..count)
                .map(|_| {
                    let len = rng.random_range(1..=24);
                    (0..len)
                        .map(|_| rng.random_range(0..alphabet) as u8)
                        .collect()
                })
                .collect();
            let mut input: Vec<u8> = (0..99)
                .map(|_| rng.random_range(0..alphabet) as u8)
                .collect();
            for id in [0, count - 1] {
                let start = if id == 0 {
                    0
                } else {
                    input.len() - patterns[id].len()
                };
                input[start..start + patterns[id].len()].copy_from_slice(&patterns[id]);
            }
            for strategy in [
                Strategy::Balanced,
                Strategy::LengthSorted,
                Strategy::DynamicProgramming,
            ] {
                compare(patterns.clone(), &input, count.min(3), 8, strategy);
            }
        }
    }
}
