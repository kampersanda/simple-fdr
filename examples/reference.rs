use simple_fdr::{Patterns, naive::Naive};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let patterns = Patterns::new(vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()])?;
    let matcher = Naive::new(patterns);
    let input = b"abcd ad bc";
    for m in matcher.find_all(input) {
        println!(
            "pattern {}: [{}..{}) = {:?}",
            m.pattern_id,
            m.start,
            m.end,
            &input[m.start..m.end]
        );
    }
    Ok(())
}
