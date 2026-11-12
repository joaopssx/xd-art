//! Compares xd-art against images produced by the original program's code.
//!
//! The files in `tests/golden/` come from `scripts/gen-goldens.sh`. Nearest sampling and
//! every dither mode must match exactly; the float based filters may differ by one step
//! per channel, since C and Rust do not always round `f32` math the same way.

mod common;

use common::{assert_golden, diff, golden, work_rgba};
use xd_art_core::buffer::Work;
use xd_art_core::cancel::Cancel;
use xd_art_core::pool::Pool;
use xd_art_core::sampling;
use xd_art_core::settings::SampleMode;

/// Tolerance for float based filters, in 8-bit steps.
const FLOAT_TOL: u8 = 1;

fn sampled(src: &Work, w: u32, h: u32, mode: SampleMode, off: (f32, f32)) -> Work {
    sampling::sample(src, w, h, mode, off, &Pool::new(), &Cancel::new()).unwrap()
}

fn check_sampling(mode: SampleMode, tol: u8) {
    let noise = common::noise().to_work();
    let grad = common::gradient().to_work();
    let name = mode.name();
    let a = sampled(&noise, 17, 13, mode, (0.0, 0.0));
    assert_golden(&format!("sample_{name}_noise"), 17, 13, &work_rgba(&a), tol);
    let b = sampled(&grad, 70, 50, mode, (0.3, -0.2));
    assert_golden(
        &format!("sample_{name}_up_offset"),
        70,
        50,
        &work_rgba(&b),
        tol,
    );
}

#[test]
fn sampling_nearest() {
    check_sampling(SampleMode::Nearest, 0);
}

#[test]
fn sampling_bilinear() {
    check_sampling(SampleMode::Bilinear, FLOAT_TOL);
}

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

#[test]
fn sampling_bicubic() {
    check_sampling(SampleMode::Bicubic, FLOAT_TOL);
}

#[test]
fn sampling_lanczos() {
    check_sampling(SampleMode::Lanczos, FLOAT_TOL);
}

#[test]
fn sampling_cluster() {
    check_sampling(SampleMode::Cluster, 0);
}

#[test]
fn blur_matches_the_original_exactly() {
    use xd_art_core::prep;
    for (name, radius) in [("blur_0_5", 0.5), ("blur_1_5", 1.5), ("blur_4_25", 4.25)] {
        let mut img = common::noise().to_work();
        prep::box_blur(&mut img, radius, &Pool::new(), &Cancel::new()).unwrap();
        assert_golden(name, 48, 40, &work_rgba(&img), 0);
    }
}

fn adjusted(f: impl FnOnce(&mut Work)) -> Vec<u8> {
    let mut img = sampled(
        &common::noise().to_work(),
        24,
        20,
        SampleMode::Nearest,
        (0.0, 0.0),
    );
    f(&mut img);
    work_rgba(&img)
}

#[test]
fn adjustments_match_the_original() {
    use xd_art_core::prep;
    use xd_art_core::settings::Adjust;
    let (pool, c) = (Pool::new(), Cancel::new());
    let a = adjusted(|i| prep::sharpen(i, 0.8, &pool, &c).unwrap());
    assert_golden("adjust_sharpen", 24, 20, &a, 0);
    let set = Adjust {
        hue: 40.0,
        saturation: 1.3,
        contrast: 1.2,
        brightness: 0.05,
        ..Adjust::default()
    };
    let a = adjusted(|i| prep::hscb(i, &set, &c).unwrap());
    assert_golden("adjust_hscb", 24, 20, &a, FLOAT_TOL);
    let set = Adjust {
        saturation: 0.4,
        contrast: 0.7,
        brightness: -0.1,
        ..Adjust::default()
    };
    let a = adjusted(|i| prep::hscb(i, &set, &c).unwrap());
    assert_golden("adjust_hscb_nohue", 24, 20, &a, FLOAT_TOL);
    let a = adjusted(|i| prep::gamma(i, 1.6, &c).unwrap());
    assert_golden("adjust_gamma", 24, 20, &a, FLOAT_TOL);
    let a = adjusted(|i| prep::tint(i, [200, 150, 255], 1.0, &c).unwrap());
    assert_golden("adjust_tint", 24, 20, &a, 0);
}

/// Closest color per pixel, the original's "no dithering" path.
fn closest_rgba(img: &Work, metric: xd_art_core::settings::Metric) -> Vec<u8> {
    use xd_art_core::buffer::to8;
    use xd_art_core::color::Matcher;
    let m = Matcher::new(&common::PICO8, metric);
    let mut out = Vec::new();
    for p in &img.px {
        if sampling::is_clear(*p, 128) {
            out.extend([0; 4]);
        } else {
            let c = m.closest([to8(p[0]), to8(p[1]), to8(p[2])]);
            out.extend(common::PICO8[c as usize]);
            out.push(255);
        }
    }
    out
}

#[test]
fn metrics_pick_the_same_colors_as_the_original() {
    use xd_art_core::settings::Metric;
    let src = sampled(
        &common::gradient().to_work(),
        32,
        24,
        SampleMode::Bilinear,
        (0.0, 0.0),
    );
    for m in Metric::ALL {
        assert_golden(
            &format!("metric_{}", m.name()),
            32,
            24,
            &closest_rgba(&src, m),
            0,
        );
    }
}

fn dithered(
    src: &Work,
    mode: xd_art_core::settings::DitherMode,
    amount: f32,
    metric: xd_art_core::settings::Metric,
) -> Vec<u8> {
    use xd_art_core::dither;
    use xd_art_core::settings::Dither;
    let d = Dither {
        mode,
        amount,
        ..Dither::default()
    };
    let out = dither::map(
        src,
        &common::PICO8,
        metric,
        &d,
        &Pool::new(),
        &Cancel::new(),
    )
    .unwrap();
    out.to_image(&common::PICO8).unwrap().to_rgba()
}

fn dither_case(mode: xd_art_core::settings::DitherMode) {
    use xd_art_core::settings::Metric;
    let src = sampled(
        &common::gradient().to_work(),
        32,
        24,
        SampleMode::Bilinear,
        (0.0, 0.0),
    );
    assert_golden(
        &format!("dither_{}", mode.name()),
        32,
        24,
        &dithered(&src, mode, 0.4, Metric::Redmean),
        0,
    );
}

#[test]
fn ordered_dithering_matches_the_original() {
    use xd_art_core::settings::{DitherMode, Metric};
    for mode in DitherMode::ALL
        .into_iter()
        .filter(|m| m.is_ordered() || *m == DitherMode::None)
    {
        dither_case(mode);
    }
    let src = sampled(
        &common::noise().to_work(),
        32,
        24,
        SampleMode::Nearest,
        (0.0, 0.0),
    );
    let out = dithered(&src, DitherMode::Bayer4, 0.9, Metric::Rgb);
    assert_golden("dither_bayer4x4_alpha", 32, 24, &out, 0);
}

#[test]
fn error_diffusion_and_picocad_match_the_original() {
    use xd_art_core::settings::{DitherMode, Metric};
    for mode in DitherMode::ALL
        .into_iter()
        .filter(|m| m.is_diffusion() || *m == DitherMode::Picocad)
    {
        dither_case(mode);
    }
    let src = sampled(
        &common::noise().to_work(),
        32,
        24,
        SampleMode::Nearest,
        (0.0, 0.0),
    );
    let out = dithered(&src, DitherMode::Floyd, 0.4, Metric::Redmean);
    assert_golden("dither_floyd_alpha", 32, 24, &out, 0);
}
