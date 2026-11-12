//! Compares xd-art against images produced by the original program's code.
//!
//! The files in `tests/golden/` come from `scripts/gen-goldens.sh`. Nearest sampling and
//! every dither mode must match exactly; the float based filters may differ by one step
//! per channel, since C and Rust do not always round `f32` math the same way.

mod common;

use common::{assert_golden, diff, golden};

#[test]
fn fixtures_match_the_harness() {
    for (name, img) in [
        ("in_gradient", common::gradient()),
        ("in_noise", common::noise()),
    ] {
        let (w, h, rgba) = golden(name);
        assert_eq!((w, h), (img.width(), img.height()));
        assert_golden(name, w, h, &img.to_rgba(), 0);
        assert_eq!(rgba, img.to_rgba());
    }
}

#[test]
fn diff_reports_count_max_and_first() {
    let a = [0, 0, 0, 255, 10, 10, 10, 255, 20, 20, 20, 255];
    let b = [0, 0, 0, 255, 10, 12, 10, 255, 20, 20, 25, 255];
    let d = diff(3, &a, &b, 1);
    assert_eq!(d.count, 2);
    assert_eq!(d.max, 5);
    assert_eq!(d.first, Some((1, 0, [10, 10, 10, 255], [10, 12, 10, 255])));
    let text = d.to_string();
    assert!(
        text.starts_with("2 of 12 channels off, max diff 5, first at (1, 0)"),
        "{text}"
    );
    assert_eq!(diff(3, &a, &b, 5).count, 0);
}
