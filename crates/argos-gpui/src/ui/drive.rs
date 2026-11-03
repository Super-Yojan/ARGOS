//! Teleop: the Drive section, turning limit chips, and the W/A/S/D pad.

use super::widgets::hint;
use super::widgets::section;
use crate::Dashboard;

use argos_gpui::RoverMark;
use argos_gpui::console::{self};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonGroup},
};
use gpui_kit::{prelude::*, *};
use std::collections::BTreeSet;

/// Manual drive settings and the on-screen keys. Shown only while this
/// window drives the rover.
pub(super) fn drive_section(
    dashboard: &Dashboard,
    mark: &RoverMark,
    limits: argos_core::TeleopLimits,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let speed = console::speed_index(limits.linear);
    section("Drive · manual", cx)
        .child(
            ButtonGroup::new("drive-speed")
                .outline()
                .small()
                .children(
                    console::DRIVE_SPEEDS
                        .iter()
                        .enumerate()
                        .map(|(index, value)| {
                            Button::new(("speed", index as u64))
                                .label(format!("{value:.1} m/s"))
                                .selected(speed == Some(index))
                        }),
                )
                .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                    if let Some(value) = clicks
                        .first()
                        .and_then(|index| console::DRIVE_SPEEDS.get(*index))
                        && let Err(error) = this.feed.set_linear(*value)
                    {
                        this.note = error;
                    }
                    cx.notify();
                })),
        )
        .when(
            mark.requested_level.as_deref() == Some("teleop") && mark.status == "pose stale",
            |section| {
                section.child(hint(
                    "Camera pose unavailable · manual control does not need tracking",
                    cx,
                ))
            },
        )
        .child(turning_row(limits, cx))
        .child(autonomy_turning_row(mark, cx))
        .child(hint(
            "Hold W/A/S/D to drive. Release to stop. Reverse uses the same turning radius.",
            cx,
        ))
        .child(drive_pad(&dashboard.held, cx))
}

fn turning_row(limits: argos_core::TeleopLimits, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let yaw = cycle(limits.yaw_rate, &[0.25, 0.5, 1.0]);
    let accel = cycle(limits.yaw_accel, &[0.25, 0.5, 1.0]);
    let decel = cycle(limits.yaw_decel, &[0.4, 0.8, 1.6]);
    let radius = cycle(limits.min_turn_radius, &[0.35, 0.5, 1.0]);
    h_flex()
        .flex_wrap()
        .gap_1()
        .child(limit_chip(
            "yaw-rate",
            format!("Yaw {:.2}", limits.yaw_rate),
            cx,
            move |this| this.feed.set_yaw_rate(yaw),
        ))
        .child(limit_chip(
            "yaw-accel",
            format!("Accel {:.2}", limits.yaw_accel),
            cx,
            move |this| this.feed.set_yaw_accel(accel),
        ))
        .child(limit_chip(
            "yaw-decel",
            format!("Decel {:.2}", limits.yaw_decel),
            cx,
            move |this| this.feed.set_yaw_decel(decel),
        ))
        .child(limit_chip(
            "turn-radius",
            format!("Radius {:.2}", limits.min_turn_radius),
            cx,
            move |this| this.feed.set_min_radius(radius),
        ))
        .child(
            Button::new("spin")
                .outline()
                .small()
                .selected(limits.allow_spin)
                .label(if limits.allow_spin {
                    "In-place spin on"
                } else {
                    "In-place spin off"
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    let allow = !this.feed.limits().allow_spin;
                    if let Err(error) = this.feed.set_spin(allow) {
                        this.note = error;
                    }
                    cx.notify();
                })),
        )
}

/// Terra's planner turning and explore-through-unknown, which govern autonomy.
/// Teleop spin above only shapes the operator's own commands. Sending either
/// also sets CrabTrapBot's size.
fn autonomy_turning_row(mark: &RoverMark, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let id = mark.id;
    let reported = argos_core::navigation_allows_in_place(&mark.navigation_settings);
    let through = argos_core::navigation_explores_through_unknown(&mark.navigation_settings);
    let footprint = mark.navigation_settings["footprint_radius"].as_f64();
    h_flex()
        .flex_wrap()
        .items_center()
        .gap_1()
        .child(
            Button::new("autonomy-spin")
                .outline()
                .small()
                .selected(reported == Some(true))
                .label(match reported {
                    Some(true) => "Autonomy spin on",
                    Some(false) => "Autonomy spin off",
                    None => "Autonomy spin unknown",
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    let allow = reported != Some(true);
                    this.feed.configure_navigation(
                        id,
                        argos_core::NavigationEdit::crab_trap_bot(allow),
                        format!(
                            "Requested autonomy in-place turns {} with CrabTrapBot size",
                            if allow { "on" } else { "off" }
                        ),
                    );
                    cx.notify();
                })),
        )
        .child(
            Button::new("explore-through-unknown")
                .outline()
                .small()
                .selected(through == Some(true))
                .disabled(through.is_none())
                .label(match through {
                    Some(true) => "Explore through unknown on",
                    Some(false) => "Explore through unknown off",
                    None => "Explore through unknown needs Terra update",
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    let on = through != Some(true);
                    this.feed.configure_navigation(
                        id,
                        argos_core::NavigationEdit {
                            explore_through_unknown: Some(on),
                            ..argos_core::NavigationEdit::crab_trap_bot(reported == Some(true))
                        },
                        format!(
                            "Requested L4 Explore through unknown space {}",
                            if on { "on" } else { "off" }
                        ),
                    );
                    cx.notify();
                })),
        )
        .child(hint(
            format!(
                "{}sets CrabTrapBot {:.2} × {:.2} m",
                footprint.map_or(String::new(), |r| format!("Footprint {r:.2} m · ")),
                argos_core::CRAB_TRAP_BOT_LENGTH,
                argos_core::CRAB_TRAP_BOT_WIDTH,
            ),
            cx,
        ))
}

/// A drive limit that steps to its next preset on each click.
fn limit_chip(
    id: &'static str,
    label: String,
    cx: &mut Context<Dashboard>,
    apply: impl Fn(&mut Dashboard) -> Result<(), String> + 'static,
) -> impl IntoElement {
    Button::new(id)
        .outline()
        .small()
        .compact()
        .label(label)
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Err(error) = apply(this) {
                this.note = error;
            }
            cx.notify();
        }))
}

fn cycle(current: f64, choices: &[f64]) -> f64 {
    choices
        .iter()
        .copied()
        .find(|value| *value > current + 0.001)
        .unwrap_or(choices[0])
}

/// On-screen W/A/S/D. Each key drives while the pointer holds it, like the
/// keyboard; a held key shows as selected.
fn drive_pad(held: &BTreeSet<String>, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let key = |name: &'static str, label: &'static str, cx: &mut Context<Dashboard>| {
        Button::new(name)
            .outline()
            .small()
            .w(px(36.))
            .label(label)
            .selected(held.contains(name))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.hold(name, true);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.hold(name, false);
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.hold(name, false);
                    cx.notify();
                }),
            )
    };
    v_flex()
        .gap_1()
        .items_center()
        .child(key("w", "W", cx))
        .child(
            h_flex()
                .gap_1()
                .child(key("a", "A", cx))
                .child(key("s", "S", cx))
                .child(key("d", "D", cx)),
        )
}

pub(super) fn drive_key(key: &str) -> Option<&'static str> {
    match key {
        "w" | "up" => Some("w"),
        "s" | "down" => Some("s"),
        "a" | "left" => Some("a"),
        "d" | "right" => Some("d"),
        _ => None,
    }
}
