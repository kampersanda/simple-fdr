use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use simple_fdr::{
    Patterns,
    fdr_filter::{BatchSize, FdrFilter, SuperCharacterBits, Verification},
    grouping::{Coefficients, Strategy},
    naive::Naive,
    simd::Backend,
};
use std::num::NonZeroUsize;

fn compare(bytes: Vec<Vec<u8>>, input: &[u8], q: usize, length: usize) {
    let patterns = Patterns::new(bytes).unwrap();
    let expected = Naive::new(patterns.clone()).find_all(input);
    for bits in [
        SuperCharacterBits::Zero,
        SuperCharacterBits::Four,
        SuperCharacterBits::Eight,
    ] {
        for verification in [Verification::Exhaustive, Verification::Hash] {
            let mut scalar = FdrFilter::new(
                patterns.clone(),
                NonZeroUsize::new(q).unwrap(),
                NonZeroUsize::new(length).unwrap(),
                Strategy::Balanced,
                Coefficients::default(),
            )
            .with_super_character(bits)
            .with_verification(verification);
            let mut candidates = Vec::new();
            scalar.scan_candidates(input, |c| candidates.push(c));
            let stats = scalar.statistics(input);
            for batch in [
                BatchSize::One,
                BatchSize::Two,
                BatchSize::Four,
                BatchSize::Eight,
            ] {
                scalar = scalar.with_batch_size(batch);
                for backend in [Backend::Integer, Backend::Neon, Backend::Sse2] {
                    if !backend.is_supported() {
                        continue;
                    }
                    scalar = scalar.with_backend(backend).unwrap();
                    let simd = &scalar;
                    let mut actual = Vec::new();
                    simd.scan_candidates(input, |c| actual.push(c));
                    assert_eq!(
                        actual, candidates,
                        "q={q} length={length} bits={bits:?} batch={batch:?} backend={backend:?}"
                    );
                    let mut found = simd.find_all(input);
                    found.sort_unstable();
                    assert_eq!(found, expected);
                    assert_eq!(simd.statistics(input), stats);
                }
            }
        }
    }
}

#[test]
fn boundaries_eof_all_bytes_and_duplicate_ids() {
    compare(vec![], b"abc", 8, 8);
    compare(vec![b"ab".to_vec()], b"", 8, 8);
    for start in 0..8 {
        for tail in 0..8 {
            let mut input = vec![b'x'; start];
            input.extend_from_slice(b"ababaaa");
            input.extend(vec![b'x'; tail]);
            compare(
                vec![
                    b"ab".to_vec(),
                    b"ab".to_vec(),
                    b"a".to_vec(),
                    b"aba".to_vec(),
                    vec![b'x'; 40],
                ],
                &input,
                2,
                8,
            );
        }
    }
    let all: Vec<u8> = (0..=255).collect();
    compare(vec![all.clone()], &all, 1, 129);
    compare(all.iter().map(|&b| vec![b]).collect(), &all, 129, 8);
    for q in [8, 31, 32, 63, 64, 65, 127, 128, 129] {
        compare(
            (0..q * 2)
                .map(|id| vec![(id % 3) as u8; 7 + id % 3])
                .collect(),
            &[0; 25],
            q,
            9,
        );
    }
}

#[test]
fn fixed_seed_random_differential() {
    let mut rng = ChaCha8Rng::seed_from_u64(83);
    for alphabet in [4u16, 256] {
        for count in [1, 17, 65] {
            let patterns: Vec<Vec<u8>> = (0..count)
                .map(|_| {
                    (0..rng.random_range(1..=24))
                        .map(|_| rng.random_range(0..alphabet) as u8)
                        .collect()
                })
                .collect();
            let mut input: Vec<u8> = (0..97)
                .map(|_| rng.random_range(0..alphabet) as u8)
                .collect();
            for pattern in &patterns {
                let start = rng.random_range(0..=input.len() - pattern.len());
                input[start..start + pattern.len()].copy_from_slice(pattern);
            }
            compare(patterns, &input, count, 17);
        }
    }
}

#[test]
fn unsupported_backends_are_rejected() {
    for backend in [Backend::Integer, Backend::Neon, Backend::Sse2] {
        let filter = FdrFilter::new(
            Patterns::new(vec![]).unwrap(),
            NonZeroUsize::new(8).unwrap(),
            NonZeroUsize::new(8).unwrap(),
            Strategy::Balanced,
            Coefficients::default(),
        );
        assert_eq!(filter.with_backend(backend).is_ok(), backend.is_supported());
    }
}
