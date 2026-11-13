//! Startup trace and the timing overlay.
//!
//! Set `XD_ART_TRACE=1` to print one line per startup step to stderr, or set it to a
//! file path to append the lines there:
//!
//! ```text
//! xd-art trace: platform         259.86 ms
//! xd-art trace: config           260.25 ms
//! xd-art trace: window           277.38 ms
//! xd-art trace: first frame      282.56 ms
//! xd-art trace: decoded          286.37 ms
//! xd-art trace: pipeline      blur 0.0 sample 1.0 adjust 0.0 palette 0.0 map 0.5 post 0.0 output 0.0
//! xd-art trace: image shown      291.16 ms
//! ```
//!
//! Times count from the start of `main`; whatever the system does before that (loading
//! the executable and its libraries) is not in them. `XD_ART_TRACE_EXIT=frame` quits
//! after the first frame and `XD_ART_TRACE_EXIT=image` after the first image is shown,
//! which is what `scripts/measure-startup.sh` uses to time whole runs.
//!
//! Ctrl+Shift+D shows the same numbers over the preview, trace or not. No tracing crate:
//! a handful of `Instant`s is all this needs.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use gpui::{AnyElement, div, prelude::*, px};
use xd_art_core::pipeline::{Report, Stage};

use crate::Marks;
use crate::app::Root;
use crate::theme::{self, Palette};

enum Sink {
    Off,
    Stderr,
    File(PathBuf),
}

fn sink() -> &'static Sink {
    static SINK: OnceLock<Sink> = OnceLock::new();
    SINK.get_or_init(|| match std::env::var_os("XD_ART_TRACE") {
        None => Sink::Off,
        Some(v) if v.is_empty() || v == "0" => Sink::Off,
        Some(v) if v == "1" => Sink::Stderr,
        Some(v) => Sink::File(PathBuf::from(v)),
    })
}

/// Whether trace lines are written.
pub fn enabled() -> bool {
    !matches!(sink(), Sink::Off)
}

/// When to quit by itself, from `XD_ART_TRACE_EXIT`: `"frame"` or `"image"`.
pub fn exit_after() -> Option<&'static str> {
    static EXIT: OnceLock<Option<&'static str>> = OnceLock::new();
    *EXIT.get_or_init(|| match std::env::var("XD_ART_TRACE_EXIT").as_deref() {
        Ok("frame") => Some("frame"),
        Ok("image") => Some("image"),
        _ => None,
    })
}

/// Writes one trace line. Errors writing it are ignored: the trace must never break
/// the program it measures.
pub fn line(text: &str) {
    match sink() {
        Sink::Off => {}
        Sink::Stderr => eprintln!("xd-art trace: {text}"),
        Sink::File(p) => {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
            {
                let _ = writeln!(f, "xd-art trace: {text}");
            }
        }
    }
}

/// Traces `name` at `at`, counted from `start`.
pub fn mark(start: Instant, name: &str, at: Instant) {
    if enabled() {
        line(&step(name, start, at));
    }
}

fn step(name: &str, start: Instant, at: Instant) -> String {
    format!(
        "{name:<14}{:>9.2} ms",
        at.duration_since(start).as_secs_f64() * 1000.0
    )
}

/// The stage times of one conversion, in pipeline order, for stages that ran.
pub fn stages(report: &Report) -> String {
    Stage::ALL
        .iter()
        .filter(|s| report.ran.contains(**s))
        .map(|s| {
            format!(
                "{} {:.1}",
                s.name(),
                report.times[*s as usize].as_secs_f64() * 1000.0
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Marks {
    /// Every step reached so far, as `(name, time)`, in order.
    pub fn steps(&self) -> Vec<(&'static str, Instant)> {
        [
            ("platform", self.platform),
            ("config", self.config),
            ("window", self.window),
            ("first frame", self.first_frame),
            ("decoded", self.image),
            ("image shown", self.shown),
        ]
        .into_iter()
        .filter_map(|(n, t)| t.map(|t| (n, t)))
        .collect()
    }
}

impl Root {
    /// Records that a converted image was shown for the first time.
    pub(crate) fn mark_shown(&mut self, cx: &mut gpui::Context<Self>) {
        if self.marks.shown.is_some() {
            return;
        }
        let now = Instant::now();
        self.marks.shown = Some(now);
        mark(self.marks.start, "image shown", now);
        if exit_after() == Some("image") {
            cx.quit();
        }
    }

    /// The timing overlay.
    pub(crate) fn dev_overlay(&self, t: Palette) -> AnyElement {
        let mut rows: Vec<String> = self
            .marks
            .steps()
            .into_iter()
            .map(|(n, at)| step(n, self.marks.start, at))
            .collect();
        if let Some((took, report)) = &self.last_run {
            rows.push(format!(
                "last run      {:>9.2} ms",
                took.as_secs_f64() * 1000.0
            ));
            rows.push(stages(report));
        }
        div()
            .absolute()
            .top(px(48.0))
            .right(px(12.0))
            .p_2()
            .rounded(px(4.0))
            .bg(t.panel)
            .border_1()
            .border_color(t.border)
            .font(theme::number_font())
            .text_size(theme::size::small())
            .text_color(t.text)
            .flex()
            .flex_col()
            .children(rows.into_iter().map(|r| div().child(r)))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn steps_keep_order_and_skip_missing() {
        let start = Instant::now();
        let m = Marks {
            start,
            platform: None,
            config: Some(start),
            window: Some(start + Duration::from_millis(30)),
            first_frame: None,
            image: Some(start + Duration::from_millis(50)),
            shown: None,
        };
        let names: Vec<&str> = m.steps().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["config", "window", "decoded"]);
        assert_eq!(
            step("window", start, start + Duration::from_millis(30)),
            "window            30.00 ms"
        );
    }
}
