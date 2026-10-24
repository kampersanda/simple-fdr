use std::num::NonZeroUsize;

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use simple_fdr::{
    Patterns,
    filter::{Candidate, Filter, SearchStats},
    naive::Naive,
};

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

fn compare(patterns: Vec<Vec<u8>>, input: &[u8], buckets: usize, length: Option<usize>) {
    let patterns = Patterns::new(patterns).unwrap();
    let expected = Naive::new(patterns.clone()).find_all(input);
    let filter = Filter::new(patterns, nz(buckets), length.map(nz));
    let mut found = filter.find_all(input);
    let mut scanned = Vec::new();
    filter.scan(input, |m| scanned.push(m));
    assert_eq!(found, scanned);
    assert_eq!(filter.find_all(input), scanned);
    assert!(
        found
            .windows(2)
            .all(|p| (p[0].end, p[0].pattern_id) < (p[1].end, p[1].pattern_id))
    );
    found.sort_unstable();
    assert_eq!(found, expected);
}

fn article(buckets: usize, length: Option<usize>) -> Filter {
    Filter::new(
        Patterns::new(vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()]).unwrap(),
        nz(buckets),
        length.map(nz),
    )
}

#[test]
fn article_examples_reject_false_positives() {
    let grouping = article(2, None);
    assert_eq!(grouping.buckets(), [vec![0, 1], vec![2]]);
    assert_eq!(grouping.accepting_lengths(), [vec![2, 4], vec![2]]);
    assert_eq!(grouping.state_width(), 10);
    for input in [b"ab".as_slice(), b"adcd"] {
        let mut candidates = Vec::new();
        grouping.scan_candidates(input, |c| candidates.push(c));
        assert!(candidates.contains(&Candidate {
            bucket_id: 0,
            end: input.len()
        }));
        let mut found = grouping.find_all(input);
        found.sort_unstable();
        assert_eq!(
            found,
            Naive::new(
                Patterns::new(vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()]).unwrap()
            )
            .find_all(input)
        );
        assert!(grouping.statistics(input).false_positive_candidates > 0);
    }
    let truncation = article(3, Some(2));
    assert_eq!(truncation.state_width(), 9);
    assert!(truncation.find_all(b"acd").is_empty());
    assert_eq!(
        truncation.statistics(b"acd"),
        SearchStats {
            hash_lookups: 0,
            candidate_count: 1,
            full_comparisons: 0,
            false_positive_candidates: 1
        }
    );
    // The same suffix with enough preceding input performs a full comparison.
    assert_eq!(truncation.statistics(b"xxcd").full_comparisons, 1);
    assert!(truncation.find_all(b"xxcd").is_empty());
    let combined = article(2, Some(2));
    assert_eq!(combined.state_width(), 6);
    assert!(combined.find_all(b"cd").is_empty());
    assert_eq!(
        combined.statistics(b"cd"),
        SearchStats {
            hash_lookups: 0,
            candidate_count: 1,
            full_comparisons: 1,
            false_positive_candidates: 1
        }
    );
    for filter in [grouping, truncation, combined] {
        assert_eq!(filter.find_all(b"abcdadbc").len(), 4);
    }
}

#[test]
fn candidate_events_are_deduplicated_and_not_match_counts() {
    let filter = Filter::new(
        Patterns::new(vec![b"a".to_vec(), b"aa".to_vec(), b"aa".to_vec()]).unwrap(),
        nz(1),
        None,
    );
    let mut events = Vec::new();
    filter.scan_candidates(b"aa", |c| events.push(c));
    assert_eq!(
        events,
        [
            Candidate {
                bucket_id: 0,
                end: 1
            },
            Candidate {
                bucket_id: 0,
                end: 2
            }
        ]
    );
    assert_eq!(filter.find_all(b"aa").len(), 4);
    assert_eq!(
        filter.statistics(b"aa"),
        SearchStats {
            hash_lookups: 0,
            candidate_count: 2,
            full_comparisons: 4,
            false_positive_candidates: 0
        }
    );
}

#[test]
fn empty_inputs_mixed_lengths_duplicates_and_all_bytes() {
    let bytes: Vec<u8> = (0..=255).collect();
    for length in [None, Some(1), Some(2), Some(64), Some(300)] {
        for buckets in [1, 2, 64, 65, 129, 300] {
            compare(vec![], b"abc", buckets, length);
            compare(vec![b"a".to_vec()], b"", buckets, length);
            compare(
                vec![
                    b"a".to_vec(),
                    b"aa".to_vec(),
                    b"aa".to_vec(),
                    b"aaaa".to_vec(),
                ],
                b"aaa",
                buckets,
                length,
            );
            compare(
                bytes.iter().map(|&b| vec![b]).collect(),
                &bytes,
                buckets,
                length,
            );
            compare(vec![bytes.clone()], &bytes, buckets, length);
        }
    }
    let empty = Filter::new(Patterns::new(vec![]).unwrap(), nz(2), None);
    assert_eq!(empty.state_width(), 0);
    assert_eq!(empty.word_count(), 0);
    assert_eq!(empty.statistics(b"abc"), SearchStats::default());
}

#[test]
fn word_boundaries_with_active_carries_and_multiple_accepting_lengths() {
    for count in [1, 2, 3, 63, 64, 65, 127, 128, 129] {
        let patterns: Vec<Vec<u8>> = (0..count * 2).map(|id| vec![0; 63 + id % 3]).collect();
        for length in [None, Some(1), Some(64), Some(65)] {
            compare(patterns.clone(), &[0; 70], count, length);
        }
    }
}

#[test]
fn fixed_seed_differential_search_with_planted_matches() {
    let mut rng = ChaCha8Rng::seed_from_u64(31);
    for alphabet in [4u16, 256] {
        for count in [1, 3, 17, 65, 129] {
            for _ in 0..6 {
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
                for buckets in [1, 2, count] {
                    for length in [None, Some(1), Some(4), Some(64)] {
                        compare(patterns.clone(), &input, buckets, length);
                    }
                }
            }
        }
    }
}
