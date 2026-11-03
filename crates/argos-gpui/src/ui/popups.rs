//! Popups over the map: bucket dump approval and the click-to-goal confirm.

use crate::Dashboard;

use argos_core::GoalDrag;
use argos_gpui::RoverMark;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt, ThemeStyled,
    button::{Button, ButtonVariants},
};
use gpui_kit::{prelude::*, *};

/// One pop-up per rover whose dock is aligned and waiting for the operator
/// (L0–L3). L4 and L5 dump on their own, so they never ask.
pub(super) fn dump_popups(marks: &[RoverMark], cx: &mut Context<Dashboard>) -> impl IntoElement {
    let waiting: Vec<(u64, argos_core::DockReport)> = marks
        .iter()
        .filter_map(|mark| {
            mark.dock
                .clone()
                .filter(|dock| dock.awaiting_approval && dock.phase == "aligned")
                .map(|dock| (mark.id, dock))
        })
        .collect();
    v_flex()
        .absolute()
        .top_3()
        .left_0()
        .right_0()
        .items_center()
        .gap_2()
        .children(waiting.into_iter().map(|(id, dock)| {
            let dock_id = dock.dock_id;
            let detail = format!(
                "{} · {:+.2} m side · {:+.2} m range · {:+.0}°{}",
                dock.target_label(),
                dock.lateral_m,
                dock.range_m,
                dock.heading_deg,
                if dock.verified {
                    ""
                } else {
                    " · camera lost the bucket at the end, check the FPV"
                }
            );
            v_flex()
                .id(("dump-popup", id))
                .w(px(340.))
                .p_3()
                .gap_2()
                .popover_style(cx)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(format!("Rover {id} is lined up. Dump now?")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(detail),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new(("dump-popup-cancel", id))
                                .ghost()
                                .small()
                                .label("Cancel dock")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.feed.cancel_dock(id);
                                    this.note = "Dock cancel requested".into();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(("dump-popup-dump", id))
                                .primary()
                                .small()
                                .label("Open gate and dump")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.feed.approve_dump(id, dock_id);
                                    this.note = "Dump approved".into();
                                    cx.notify();
                                })),
                        ),
                )
        }))
}

pub(super) fn goal_popup(
    goal: &GoalDrag,
    at: Point<Pixels>,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let distance = match goal.arrival_radius_m {
        Some(radius) => format!(
            "{:.1} m GPS · arrival radius {radius:.0} m",
            goal.distance_m()
        ),
        None => format!("{:.1} m from rover {}", goal.distance_m(), goal.id),
    };
    div()
        .absolute()
        .left(at.x)
        .top(at.y)
        .size_0()
        .child(
            v_flex()
                .id("goal-confirm")
                .absolute()
                .bottom_3()
                .left(px(-96.))
                .w(px(192.))
                .p_2()
                .gap_2()
                .popover_style(cx)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    v_flex()
                        .child(div().text_sm().child("Send goal here?"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(distance),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("goal-cancel")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_drag();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("goal-send")
                                .primary()
                                .small()
                                .label("Send goal")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.release_drag();
                                    cx.notify();
                                })),
                        ),
                ),
        )
        .child(
            div()
                .absolute()
                .left(px(-4.))
                .top(px(-4.))
                .size_2()
                .rounded_full()
                .bg(cx.theme().success),
        )
}
