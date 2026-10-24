use simple_fdr::{Patterns, filter::Filter};
use std::num::NonZeroUsize;

fn main() {
    for (name, buckets, length) in [
        ("grouping", 2, None),
        ("truncation", 3, NonZeroUsize::new(2)),
        ("grouping-truncation", 2, NonZeroUsize::new(2)),
    ] {
        let patterns =
            Patterns::new(vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()]).unwrap();
        let filter = Filter::new(patterns, NonZeroUsize::new(buckets).unwrap(), length);
        println!(
            "{name}: {} bits, buckets={:?}, lengths={:?}",
            filter.state_width(),
            filter.buckets(),
            filter.accepting_lengths()
        );
        for input in [
            b"ab".as_slice(),
            b"adcd",
            b"acd",
            b"xxcd",
            b"cd",
            b"abcdadbc",
        ] {
            let mut candidates = Vec::new();
            filter.scan_candidates(input, |c| candidates.push(c));
            println!(
                "  {:?}: candidates={candidates:?}, matches={:?}, stats={:?}",
                String::from_utf8_lossy(input),
                filter.find_all(input),
                filter.statistics(input)
            );
        }
    }
}
