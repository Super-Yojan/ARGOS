//! The bar under the map: the selected rover, its autonomy level, and the
//! commands that act on it.

use super::levels::level_group;
use super::palette;
use super::widgets::caution;
use crate::Dashboard;

use argos_core::OPERATOR_LEVELS;
use argos_gpui::console::{self, StopOrigin};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use gpui_kit::{prelude::*, *};

pub(super) fn selected_bar(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let frame = || {
        h_flex()
            .flex_none()
            .mx_3()
            .mb_3()
            .px_4()
            .py_3()
            .gap_6()
            .items_center()
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    };
    let Some(mark) = dashboard.selected_mark() else {
        return frame()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Select a rover on the map or in the queue to see its controls."),
            )
            .into_any_element();
    };
    let id = mark.id;
    let stopped = dashboard.is_stopped(mark);
    let driving = dashboard.driving;
    let manual = mark.requested_level.as_deref() == Some("teleop");
    let level = console::level_index(mark);
    let lock = dashboard.study_lock(mark).map(str::to_owned);
    let details = dashboard.details_open;
    let stop = dashboard.stop_cause(mark);
    let callout = console::callout(mark, stop.as_ref(), driving);
    let primary = if stopped {
        Button::new(("bar-reset", id))
            .label("Reset stop")
            .on_click(cx.listener(move |this, _, window, cx| {
                this.reset_safety(id);
                this.focus.focus(window, cx);
                cx.notify();
            }))
    } else if driving {
        Button::new(("bar-leave", id))
            .label("Leave drive")
            .on_click(cx.listener(|this, _, window, cx| {
                this.stop_drive();
                this.note = "Drive off. Zero twist sent.".into();
                this.focus.focus(window, cx);
                cx.notify();
            }))
    } else {
        Button::new(("bar-drive", id))
            .label(if manual { "Drive" } else { "Take over (L0)" })
            .tooltip("Requests L0 manual control, then W/A/S/D drives while held.")
            .on_click(cx.listener(|this, _, window, cx| {
                this.begin_drive(window, cx);
                cx.notify();
            }))
    };
    frame()
        .child(
            v_flex()
                .flex_none()
                .gap_1()
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(cx.theme().muted_foreground)
                        .child("SELECTED"),
                )
                .child(div().text_lg().font_semibold().child(format!("Rover {id}")))
                .child(
                    div()
                        .max_w(px(240.))
                        .truncate()
                        .text_xs()
                        .text_color(if stopped {
                            rgb(palette::DANGER).into()
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(callout.state),
                )
                // The way out of a stop, such as the tilt to fix before Reset.
                .when_some(callout.recovery, |column, recovery| {
                    column.child(caution(recovery, cx).max_w(px(240.)))
                }),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(cx.theme().muted_foreground)
                        .child("AUTONOMY LEVEL"),
                )
                .child(level_group(id, level, lock, 0..OPERATOR_LEVELS.len(), cx))
                .child(
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(cx.theme().muted_foreground)
                        .child(console::level_line(mark)),
                ),
        )
        .child(
            v_flex()
                .flex_none()
                .gap_2()
                .child(primary.outline().small())
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(("bar-details", id))
                                .outline()
                                .small()
                                .selected(details)
                                .label("Details")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.details_open = !this.details_open;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(("bar-stop", id))
                                .danger()
                                .small()
                                .label(format!("Stop R{id}"))
                                .tooltip("Safety stop this rover (Space). Reset stop resumes.")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.stop_rover(id, StopOrigin::Button);
                                    this.focus.focus(window, cx);
                                    cx.notify();
                                })),
                        ),
                ),
        )
        .into_any_element()
}
