//! Controls drawn with plain gpui elements.
//!
//! The slider is the one most of the interface is made of. It shows its value as text
//! that can be clicked and typed into, resets on double click, moves with the arrow keys
//! (shift for bigger steps) and shows its range while it has focus.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClickEvent, Context, EventEmitter, FocusHandle, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, SharedString, Window, canvas, div, prelude::*, px,
    relative,
};

use crate::theme::{self, Theme};

/// What a slider reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliderEvent {
    /// The value changed. `live` is true while the knob is being dragged.
    Changed {
        /// New value.
        value: f32,
        /// Still dragging.
        live: bool,
    },
}

/// Range and look of a slider.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderSpec {
    /// Label on the left.
    pub label: SharedString,
    /// Smallest value.
    pub min: f32,
    /// Largest value.
    pub max: f32,
    /// Step of the arrow keys; values are rounded to it.
    pub step: f32,
    /// Value a double click goes back to.
    pub default: f32,
    /// Unit shown after the value, may be empty.
    pub unit: &'static str,
    /// Digits after the decimal point.
    pub decimals: usize,
}

/// A slider with an editable value.
pub struct Slider {
    spec: SliderSpec,
    value: f32,
    dragging: bool,
    /// Text being typed into the value field, while editing.
    editing: Option<String>,
    /// The last typed text could not be read.
    invalid: bool,
    focus: FocusHandle,
    track: Rc<Cell<Bounds<Pixels>>>,
}

impl EventEmitter<SliderEvent> for Slider {}

/// Clamps and rounds `v` to the slider's range and step.
pub fn snap(spec: &SliderSpec, v: f32) -> f32 {
    let v = v.clamp(spec.min, spec.max);
    if spec.step > 0.0 {
        let n = ((v - spec.min) / spec.step).round();
        (spec.min + n * spec.step).clamp(spec.min, spec.max)
    } else {
        v
    }
}

/// Reads typed text as a value: a number in range, with `,` accepted as the decimal
/// mark. `None` for anything else.
pub fn parse(spec: &SliderSpec, text: &str) -> Option<f32> {
    let v: f32 = text.trim().replace(',', ".").parse().ok()?;
    (v.is_finite() && v >= spec.min && v <= spec.max).then(|| snap(spec, v))
}

impl Slider {
    /// A slider at `value`.
    pub fn new(spec: SliderSpec, value: f32, cx: &mut Context<Self>) -> Self {
        Self {
            value: snap(&spec, value),
            spec,
            dragging: false,
            editing: None,
            invalid: false,
            focus: cx.focus_handle(),
            track: Rc::new(Cell::new(Bounds::default())),
        }
    }

    /// The current value.
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Sets the value without reporting it, for when settings change elsewhere.
    pub fn set_value(&mut self, v: f32, cx: &mut Context<Self>) {
        self.value = snap(&self.spec, v);
        cx.notify();
    }

    fn change(&mut self, v: f32, live: bool, cx: &mut Context<Self>) {
        let v = snap(&self.spec, v);
        if v != self.value || !live {
            self.value = v;
            cx.emit(SliderEvent::Changed { value: v, live });
            cx.notify();
        }
    }

    fn value_at(&self, x: Pixels) -> f32 {
        let b = self.track.get();
        let w = f32::from(b.size.width).max(1.0);
        let t = (f32::from(x - b.origin.x) / w).clamp(0.0, 1.0);
        self.spec.min + t * (self.spec.max - self.spec.min)
    }

    fn on_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.dragging = true;
        self.change(self.value_at(e.position.x), true, cx);
    }

    fn on_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        if e.pressed_button != Some(MouseButton::Left) {
            self.dragging = false;
            self.change(self.value, false, cx);
            return;
        }
        self.change(self.value_at(e.position.x), true, cx);
    }

    fn on_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = e.keystroke.key.as_str();
        if let Some(text) = &mut self.editing {
            match k {
                "enter" => {
                    let typed = text.clone();
                    match parse(&self.spec, &typed) {
                        Some(v) => {
                            self.editing = None;
                            self.invalid = false;
                            self.change(v, false, cx);
                        }
                        None => self.invalid = true,
                    }
                }
                "escape" => {
                    self.editing = None;
                    self.invalid = false;
                }
                "backspace" => {
                    text.pop();
                    self.invalid = false;
                }
                _ => {
                    if let Some(c) = e.keystroke.key_char.as_deref()
                        && c.chars()
                            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-'))
                    {
                        text.push_str(c);
                        self.invalid = false;
                    }
                }
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        let big = if e.keystroke.modifiers.shift {
            10.0
        } else {
            1.0
        };
        match k {
            "left" | "down" => self.change(self.value - self.spec.step * big, false, cx),
            "right" | "up" => self.change(self.value + self.spec.step * big, false, cx),
            "home" => self.change(self.spec.min, false, cx),
            "end" => self.change(self.spec.max, false, cx),
            "enter" => {
                self.editing = Some(self.text());
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn text(&self) -> String {
        format!("{:.*}", self.spec.decimals, self.value)
    }
}

impl Render for Slider {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx).c;
        let focused = self.focus.is_focused(window);
        let span = (self.spec.max - self.spec.min).max(f32::EPSILON);
        let fill = ((self.value - self.spec.min) / span).clamp(0.0, 1.0);
        let track_cell = self.track.clone();
        let shown = match &self.editing {
            Some(s) => format!("{s}|"),
            None => format!("{}{}", self.text(), self.spec.unit),
        };
        let value_color = if self.invalid {
            t.danger
        } else if self.editing.is_some() {
            t.accent
        } else {
            t.text
        };
        let spec = self.spec.clone();
        div()
            .id(SharedString::from(format!("slider-{}", self.spec.label)))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(theme::size::body())
                    .child(div().text_color(t.muted).child(self.spec.label.clone()))
                    .child(
                        div()
                            .id("value")
                            .px_1()
                            .rounded(px(3.0))
                            .font(theme::number_font())
                            .text_color(value_color)
                            .when(self.editing.is_some(), |d| d.bg(t.elevated))
                            .cursor_text()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                window.focus(&this.focus);
                                this.editing = Some(this.text());
                                cx.notify();
                            }))
                            .child(shown),
                    ),
            )
            .child(
                div()
                    .id("track")
                    .h(px(14.0))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
                    .on_mouse_move(cx.listener(Self::on_move))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if this.dragging {
                                this.dragging = false;
                                this.change(this.value, false, cx);
                            }
                        }),
                    )
                    .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                        if e.click_count() == 2 {
                            this.change(this.spec.default, false, cx);
                        }
                    }))
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(4.0))
                            .rounded(px(2.0))
                            .bg(t.border)
                            .child(
                                canvas(
                                    move |b, _: &mut Window, _: &mut App| track_cell.set(b),
                                    |_, (), _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .h_full()
                                    .w(relative(fill))
                                    .rounded(px(2.0))
                                    .bg(t.accent),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(relative(fill))
                                    .top(px(-4.0))
                                    .ml(px(-6.0))
                                    .size(px(12.0))
                                    .rounded_full()
                                    .bg(if focused || self.dragging {
                                        t.text
                                    } else {
                                        t.muted
                                    }),
                            ),
                    ),
            )
            .when(focused, |d| {
                d.child(
                    div()
                        .text_size(theme::size::small())
                        .text_color(t.muted)
                        .font(theme::number_font())
                        .child(format!(
                            "{:.*} to {:.*}{}, double click resets to {:.*}",
                            spec.decimals,
                            spec.min,
                            spec.decimals,
                            spec.max,
                            spec.unit,
                            spec.decimals,
                            spec.default
                        )),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SliderSpec {
        SliderSpec {
            label: "x".into(),
            min: 0.0,
            max: 2.0,
            step: 0.05,
            default: 1.0,
            unit: "",
            decimals: 2,
        }
    }

    #[test]
    fn snapping() {
        let s = spec();
        assert!((snap(&s, 1.02) - 1.0).abs() < 1e-6);
        assert!((snap(&s, 1.03) - 1.05).abs() < 1e-6);
        assert_eq!(snap(&s, 9.0), 2.0);
        assert_eq!(snap(&s, -1.0), 0.0);
    }

    #[test]
    fn typed_values() {
        let s = spec();
        assert_eq!(parse(&s, " 1.5 "), Some(1.5));
        assert_eq!(parse(&s, "1,5"), Some(1.5));
        assert_eq!(
            parse(&s, "3"),
            None,
            "out of range is rejected, not clamped"
        );
        assert_eq!(parse(&s, "abc"), None);
        assert_eq!(parse(&s, "NaN"), None);
    }
}
