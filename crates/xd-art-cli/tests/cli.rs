//! Runs the real binary.

use std::path::PathBuf;
use std::process::Command;

use xd_art_core::buffer::Image;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xd-art-cli"))
}

fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xd-art-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sample(dir: &std::path::Path) -> PathBuf {
    let mut img = Image::new(80, 50, [0; 3]).unwrap();
    for y in 0..50 {
        for x in 0..80 {
            img.set(x, y, [(x * 3) as u8, (y * 5) as u8, 90, 255]);
        }
    }
    let p = dir.join("in.png");
    xd_art_core::io::save(&img, &p).unwrap();
    p
}

#[test]
fn convert_writes_the_requested_size() {
    let d = workdir("size");
    let input = sample(&d);
    let out = d.join("out.png");
    let st = bin()
        .args([
            "convert",
            input.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "-w",
            "20",
            "-h",
            "10",
            "-s",
            "3",
            "-q",
        ])
        .status()
        .unwrap();
    assert!(st.success());
    let img = xd_art_core::io::load(&out).unwrap();
    assert_eq!((img.width(), img.height()), (60, 30));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn exit_codes() {
    let d = workdir("codes");
    let input = sample(&d);
    let bad_flag = bin()
        .args(["convert", input.to_str().unwrap(), "--nope"])
        .status()
        .unwrap();
    assert_eq!(bad_flag.code(), Some(2));
    let bad_value = bin()
        .args(["convert", input.to_str().unwrap(), "-d", "wobbly"])
        .status()
        .unwrap();
    assert_eq!(bad_value.code(), Some(2));
    let missing = d.join("missing.png");
    let out = d.join("o.png");
    let failed = bin()
        .args([
            "convert",
            missing.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert_eq!(failed.code(), Some(1));
    let ok = bin().args(["list", "dithers"]).output().unwrap();
    assert!(ok.status.success());
    assert!(String::from_utf8_lossy(&ok.stdout).contains("floyd"));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn refuses_to_overwrite_without_force() {
    let d = workdir("force");
    let input = sample(&d);
    let out = d.join("o.png");
    std::fs::write(&out, b"keep me").unwrap();
    let st = bin()
        .args([
            "convert",
            input.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "-q",
        ])
        .status()
        .unwrap();
    assert_eq!(st.code(), Some(1));
    assert_eq!(std::fs::read(&out).unwrap(), b"keep me");
    let st = bin()
        .args([
            "convert",
            input.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "-q",
            "-f",
        ])
        .status()
        .unwrap();
    assert!(st.success());
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn recipe_round_trip_gives_the_same_pixels() {
    let d = workdir("recipe");
    let input = sample(&d);
    let (a, b, r) = (d.join("a.png"), d.join("b.png"), d.join("r.json"));
    let s = |args: &[&str]| assert!(bin().args(args).status().unwrap().success());
    s(&[
        "convert",
        input.to_str().unwrap(),
        "-o",
        a.to_str().unwrap(),
        "-w",
        "24",
        "-d",
        "bayer8x8",
        "--hue",
        "40",
        "--save-recipe",
        r.to_str().unwrap(),
        "-q",
    ]);
    s(&[
        "convert",
        input.to_str().unwrap(),
        "-o",
        b.to_str().unwrap(),
        "--recipe",
        r.to_str().unwrap(),
        "-q",
    ]);
    assert_eq!(
        xd_art_core::io::load(&a).unwrap(),
        xd_art_core::io::load(&b).unwrap()
    );
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn batch_with_json_lines() {
    let d = workdir("batch");
    let input = sample(&d);
    std::fs::copy(&input, d.join("second.png")).unwrap();
    std::fs::write(d.join("broken.png"), b"not a png").unwrap();
    let out = d.join("out");
    let pattern = d.join("*.png");
    let o = bin()
        .args([
            "batch",
            pattern.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--json",
            "-w",
            "16",
            "--texture-names",
        ])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1), "one file is broken");
    let text = String::from_utf8_lossy(&o.stdout);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(lines.iter().filter(|l| l.contains(r#""ok":true"#)).count() == 2);
    assert!(
        lines
            .iter()
            .any(|l| l.contains(r#""ok":false"#) && l.contains("broken.png"))
    );
    assert_eq!(lines[3], r#"{"summary":true,"converted":2,"failed":1}"#);
    assert!(out.join("second_16.png").exists());
    std::fs::remove_dir_all(&d).ok();
}
