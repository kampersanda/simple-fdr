use simple_fdr::shift_or::ShiftOr;

fn show(label: &str, matcher: &ShiftOr, input: &[u8]) {
    let width = matcher.state_width();
    println!(
        "{label}, input={:?}, width={width}",
        String::from_utf8_lossy(input)
    );
    for byte in b"abc" {
        println!("M_{} = {:0width$b}", char::from(*byte), matcher.mask(*byte));
    }
    println!("D_0 = {:0width$b}", matcher.initial_state());
    for step in matcher.trace(input) {
        println!(
            "D_{} = {:0width$b}  byte={} mask={:0width$b} match={}",
            step.end,
            step.state,
            char::from(step.byte),
            step.mask,
            step.matched,
        );
    }
    println!("matches = {:?}\n", matcher.find_all(input));
}

fn main() {
    show("acbc", &ShiftOr::new(b"acbc").unwrap(), b"aacbc");
    let classes = vec![b"ac".to_vec(), b"c".to_vec(), b"b".to_vec(), b"c".to_vec()];
    let matcher = ShiftOr::from_classes(&classes).unwrap();
    show("[ac]cbc", &matcher, b"aacbc");
    show("[ac]cbc", &matcher, b"ccbc");
}
