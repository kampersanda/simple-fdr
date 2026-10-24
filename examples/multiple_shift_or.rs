use simple_fdr::{Patterns, multiple_shift_or::MultipleShiftOr};

fn bits(words: &[u64], width: usize) -> String {
    (0..width)
        .rev()
        .map(|bit| {
            if words[bit / 64] & (1u64 << (bit % 64)) == 0 {
                '0'
            } else {
                '1'
            }
        })
        .collect()
}

fn main() {
    let matcher = MultipleShiftOr::new(
        Patterns::new(vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()]).unwrap(),
    );
    let width = matcher.state_width();
    println!(
        "patterns=[abcd, ad, bc], width={width}, words={}",
        matcher.word_count()
    );
    for id in 0..3 {
        println!("accepting bit for ID {id}: {}", matcher.accepting_bit(id));
    }
    for &byte in b"abcd" {
        println!(
            "M_{} = {}",
            char::from(byte),
            bits(matcher.mask(byte), width)
        );
    }
    println!("D_0 = {}", bits(matcher.initial_state(), width));
    for step in matcher.trace(b"abcdadbc") {
        println!(
            "D_{} = {} byte={} matches={:?}",
            step.end,
            bits(&step.state, width),
            char::from(step.byte),
            step.matches
        );
    }
}
