//! Small building blocks every component uses: buttons, panels, labels.
//!
//! They are plain functions that return a `Div`, so the caller can keep
//! styling them (`.flex_1()`, `.on_click(..)`, …).

use std::time::Duration;

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::theme::{self, Tone};

/// How a button looks. The look says how much the action matters.
#[derive(Clone, Copy)]
pub enum ButtonLook {
    /// The main action: filled blue.
    Primary,
    /// An ordinary action: outlined.
    Outline,
    /// Stops a rover: filled red.
    Stop,
    /// The main action on an alert card.
    ToneFilled(Tone),
    /// The second action on an alert card.
    ToneOutline(Tone),
    /// Cannot be used right now.
    Disabled,
}

/// A clean, compact button (34 px tall). Add `.on_click(..)` to make it do something.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    look: ButtonLook,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .min_h(px(34.0))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .text_size(px(13.0))
        .child(label.into());

    let button = match look {
        ButtonLook::Primary => button
            .bg(rgb(theme::BLUE))
            .text_color(rgb(theme::INK))
            .font_weight(FontWeight::MEDIUM),
        ButtonLook::Outline => button
            .bg(rgb(theme::PANEL_MUTED))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .text_color(rgb(theme::TEXT)),
        ButtonLook::Stop => button
            .bg(rgb(theme::STOP))
            .text_color(rgb(theme::STOP_TEXT))
            .font_weight(FontWeight::SEMIBOLD),
        ButtonLook::ToneFilled(tone) => button
            .bg(rgb(tone.fill))
            .text_color(rgb(tone.ink))
            .font_weight(FontWeight::MEDIUM),
        ButtonLook::ToneOutline(tone) => button
            .bg(rgb(theme::PANEL_MUTED))
            .border_1()
            .border_color(rgb(tone.accent))
            .text_color(rgb(tone.text)),
        ButtonLook::Disabled => button
            .bg(rgb(theme::PANEL))
            .border_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .text_color(rgb(theme::TEXT_MUTED))
            .opacity(0.55),
    };

    match look {
        ButtonLook::Disabled => button,
        ButtonLook::Outline | ButtonLook::ToneOutline(_) => button
            .cursor_pointer()
            .hover(|style| style.bg(rgb(theme::BORDER))),
        _ => button.cursor_pointer().hover(|style| style.opacity(0.88)),
    }
}

/// One half of a compact segmented control, as in "Supervise | Agent".
pub fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
) -> Stateful<Div> {
    let segment = div()
        .id(id)
        .min_h(px(26.0))
        .px_3()
        .py(px(3.0))
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .text_size(px(12.0))
        .cursor_pointer()
        .child(label.into());
    if selected {
        segment
            .bg(rgb(theme::BORDER))
            .text_color(rgb(theme::TEXT))
            .font_weight(FontWeight::SEMIBOLD)
    } else {
        segment
            .text_color(rgb(theme::TEXT_MUTED))
            .hover(|style| style.text_color(rgb(theme::TEXT)))
    }
}

/// The dark recessed track around a row of [`segment`]s.
pub fn segmented() -> Div {
    h_flex()
        .p(px(2.0))
        .gap(px(2.0))
        .bg(rgb(theme::BG))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .rounded(px(8.0))
}

/// A crisp framed card that groups related things.
pub fn panel() -> Div {
    v_flex()
        .bg(rgb(theme::PANEL_MUTED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .rounded(px(8.0))
        .p_3()
        .gap_2()
}

/// A panel title.
pub fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(theme::FONT_HEADING)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(13.5))
        .text_color(rgb(theme::TEXT))
        .child(text.into())
}

/// A small upper-case label above a list or a section.
pub fn caps_label(text: &str) -> Div {
    mono(text.to_uppercase())
        .text_size(px(11.0))
        .text_color(rgb(theme::TEXT_MUTED))
}

/// Monospaced text, for numbers and states.
pub fn mono(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(theme::FONT_MONO)
        .text_size(px(12.0))
        .child(text.into())
}

/// A small grey note under a panel's content.
pub fn note(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.0))
        .text_color(rgb(theme::TEXT_MUTED))
        .child(text.into())
}

/// A "label  value" table. Each row is `(label, value, value colour)`.
pub fn facts(rows: impl IntoIterator<Item = (&'static str, String, u32)>) -> Div {
    v_flex()
        .gap_1()
        .children(rows.into_iter().map(|(label, value, color)| {
            h_flex()
                .gap_3()
                .items_start()
                .child(
                    mono(label)
                        .w(px(96.0))
                        .flex_none()
                        .text_color(rgb(theme::TEXT_MUTED)),
                )
                .child(mono(value).flex_1().min_w_0().text_color(rgb(color)))
        }))
}

/// "0:42".
pub fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
