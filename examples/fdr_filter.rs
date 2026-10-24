use simple_fdr::{
    Patterns,
    fdr_filter::FdrFilter,
    grouping::{Coefficients, Strategy},
};
use std::num::NonZeroUsize;
fn main() {
    let patterns = Patterns::new(vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()]).unwrap();
    for strategy in [
        Strategy::Balanced,
        Strategy::LengthSorted,
        Strategy::DynamicProgramming,
    ] {
        let filter = FdrFilter::new(
            patterns.clone(),
            NonZeroUsize::new(2).unwrap(),
            NonZeroUsize::new(8).unwrap(),
            strategy,
            Coefficients::default(),
        );
        println!(
            "{strategy:?}: buckets={:?}, lengths={:?}, width={}",
            filter.buckets(),
            filter.effective_lengths(),
            filter.state_width()
        );
        println!(
            "initial={:?}, mask(a)={:?}, mask(d)={:?}",
            filter.initial_state(),
            filter.mask(b'a'),
            filter.mask(b'd')
        );
        filter.scan_candidates(b"abcdadbc", |c| println!("  candidate {c:?}"));
        println!("matches={:?}", filter.find_all(b"abcdadbc"));
    }
}
