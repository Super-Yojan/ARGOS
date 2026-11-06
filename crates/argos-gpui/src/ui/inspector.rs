//! The selected rover's side panel: header, trace, Take over / Drive / Stop,
//! and the Overview tab that stacks the sections.

use super::cards::callout_tag;
use super::drive::drive_section;
use super::interaction::geographic_gate;
use super::levels::level_group;
use super::levels::study_section;
use super::sections::capture_section;
use super::sections::dock_section;
use super::sections::goal_section;
use super::task::explore_section;
use super::task::task_section;
use super::widgets::Picked;
use super::widgets::caution;
use super::widgets::hint;
use super::widgets::rows;
use super::widgets::section;
use crate::Dashboard;

use argos_core::{
    APP_UPDATE_REQUIRED, DragGate, HEADING_MAX_AGE_S, HEADING_MAX_UNCERTAINTY_DEG, OPERATOR_LEVELS,
    OutdoorReport, drag_disabled_reason, drive_block, phase_label,
};
use argos_gpui::console::{self, StopOrigin};
use argos_gpui::{RoverMark, position_label};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonGroup, ButtonVariants},
    scroll::ScrollableElement,
    tag::Tag,
};
use gpui_kit::{prelude::*, *};

/// Live telemetry the inspector draws but does not own.
pub(super) struct Facts {
    pub(super) drag: Option<(u64, f64, f64, f64, Option<f64>)>,
    pub(super) limits: argos_core::TeleopLimits,
    pub(super) reversing_key: bool,
}

/// The selected rover: identity, state, and the commands that act on it on
/// every tab; the rest of its controls are grouped by tab and section.
pub(super) fn rover_inspector(
    dashboard: &Dashboard,
    mark: &RoverMark,
    facts: Facts,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
    let stop = dashboard.stop_cause(mark);
    let stopped = stop.is_some();
    let driving = dashboard.driving;
    let callout = console::callout(mark, stop.as_ref(), driving);
    let manual = mark.requested_level.as_deref() == Some("teleop");
    let primary = if stopped {
        Button::new(("inspector-reset", id))
            .label("Reset stop")
            .on_click(cx.listener(move |this, _, window, cx| {
                this.reset_safety(id);
                this.focus.focus(window, cx);
                cx.notify();
            }))
    } else if driving {
        Button::new(("inspector-leave", id))
            .label("Leave drive")
            .on_click(cx.listener(|this, _, window, cx| {
                this.stop_drive();
                this.note = "Drive off. Zero twist sent.".into();
                this.focus.focus(window, cx);
                cx.notify();
            }))
    } else {
        Button::new(("inspector-drive", id))
            .label(if manual { "Drive" } else { "Take over" })
            .tooltip(if manual {
                "Hold W/A/S/D in this window to drive. Release stops."
            } else {
                "Requests L0 manual control. Drive is available once the rover confirms."
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.begin_drive(window, cx);
                cx.notify();
            }))
    };
    let reversing = mark.reversing || (facts.reversing_key && driving);
    let stalled = mark
        .drive
        .is_some_and(|drive| drive.backing_up() && drive.stalled());
    let body = basics(dashboard, mark, &facts, cx).into_any_element();
    v_flex()
        .size_full()
        .bg(cx.theme().background)
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            v_flex()
                .flex_none()
                .p_4()
                .pb_3()
                .gap_3()
                .border_b_1()
                .border_color(cx.theme().border)
                // Stop sits top right, on the title row, away from Take over.
                // Still one click, plus Space anywhere.
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_base()
                                        .font_semibold()
                                        .child(format!("Rover {id}")),
                                )
                                .when(reversing, |tags| {
                                    tags.child(if stalled {
                                        Tag::danger().small().child("Backing up · not moving")
                                    } else {
                                        Tag::warning().small().child("Backing up")
                                    })
                                })
                                // The Backing up tag above already says it.
                                .children(
                                    callout
                                        .tag
                                        .filter(|tag| {
                                            !(reversing
                                                && *tag
                                                    == argos_gpui::console::CalloutTag::BackingUp)
                                        })
                                        .map(callout_tag),
                                ),
                        )
                        .child(
                            Button::new(("inspector-stop", id))
                                .danger()
                                .label("Stop")
                                .tooltip("Safety stop this rover (Space). Reset stop resumes.")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.stop_rover(id, StopOrigin::Button);
                                    this.focus.focus(window, cx);
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(callout.state),
                )
                .when_some(callout.recovery, |header, recovery| {
                    header.child(caution(recovery, cx))
                })
                .child(h_flex().gap_2().child(primary.outline().small())),
        )
        .child(
            v_flex()
                .id("inspector-body")
                // Zero basis: the body takes only the height left under the
                // header, and scrolls the rest.
                .h_0()
                .flex_1()
                .min_h_0()
                .p_4()
                .gap_5()
                .child(body)
                .overflow_y_scrollbar(),
        )
}

/// Heading uncertainty and age as the rover reports them, against the limits
/// outdoor goals need.
fn heading_line(report: &OutdoorReport) -> String {
    let uncertainty = report.heading_accuracy_deg.map_or_else(
        || "uncertainty not reported".into(),
        |degrees| format!("±{degrees:.0}°"),
    );
    let age = report.heading_age_s.map_or_else(
        || "age not reported".into(),
        |age| format!("{age:.1} s old"),
    );
    format!(
        "{uncertainty} · {age} · limit ±{HEADING_MAX_UNCERTAINTY_DEG:.0}°, {HEADING_MAX_AGE_S:.0} s"
    )
}

/// What a goal drag would do now, or why it cannot start.
pub(super) fn goal_line(
    mark: &RoverMark,
    drag: Option<(u64, f64, f64, f64, Option<f64>)>,
    link: &str,
) -> String {
    let level = mark
        .effective_level
        .clone()
        .or_else(|| mark.requested_level.clone());
    let local_gate = DragGate {
        selected: true,
        link_healthy: link == "healthy",
        pose_fresh: mark.status == "online",
        degraded: mark.degraded,
        level: level.as_deref(),
        geographic: false,
        outdoor: false,
        admission: None,
    };
    let drive_reason = drive_block(&mark.hardware);
    if let Some((_, _, _, distance, radius)) = drag.filter(|item| item.0 == mark.id) {
        match radius {
            Some(radius) => format!("{distance:.1} m GPS · arrival radius {radius:.0} m"),
            None => format!("{distance:.1} m"),
        }
    } else if mark.outdoor_gps {
        match drag_disabled_reason(geographic_gate(mark, link, true)) {
            Some(reason) => reason.to_string(),
            None => drive_reason
                .unwrap_or("Drag the map marker. Release sends the GPS goal.")
                .into(),
        }
    } else {
        match drag_disabled_reason(local_gate) {
            Some(reason) => reason.to_string(),
            None => drive_reason
                .unwrap_or("Drag the rover or its goal. Release sends.")
                .into(),
        }
    }
}

/// The Rover panel body: only what an operator needs every minute. Level,
/// what a goal drag would do, found buckets, and the drive pad while driving.
fn basics(
    dashboard: &Dashboard,
    mark: &RoverMark,
    facts: &Facts,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
    let stopped = dashboard.is_stopped(mark);
    let driving = dashboard.driving;
    let (_, hardware_detail) = console::hardware_lines(mark, stopped);
    let level = console::level_index(mark);
    let lock = dashboard.study_lock(mark).map(str::to_owned);
    v_flex()
        .gap_5()
        .when(driving, |tab| {
            tab.child(drive_section(dashboard, mark, facts.limits, cx))
        })
        .child(
            section("Autonomy", cx)
                .child(hint(console::level_line(mark), cx))
                .child(level_group(id, level, lock.clone(), 0..3, cx))
                .child(level_group(id, level, lock, 3..OPERATOR_LEVELS.len(), cx)),
        )
        .child(section("Goal", cx).child(hint(goal_line(mark, facts.drag, &dashboard.link), cx)))
        .when_some(hardware_detail, |tab, detail| {
            tab.child(section("Hardware", cx).child(caution(detail.to_owned(), cx)))
        })
        .when(!mark.objects.is_empty(), |tab| {
            let docking = mark.dock.as_ref().is_some_and(|dock| dock.active());
            tab.child(
                section("Found objects", cx).children(mark.objects.iter().map(|object| {
                    let object_id = object.object.id;
                    let bucket = object.object.class == "bucket";
                    // The line wraps so a long GPS fix cannot push Dock out of the panel.
                    h_flex()
                        .gap_2()
                        .justify_between()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .child(console::object_line(object)),
                        )
                        .when(bucket, |row| {
                            row.child(
                                div().flex_none().child(
                                    Button::new(("dock-start", id * 1000 + u64::from(object_id)))
                                        .outline()
                                        .small()
                                        .label("Dock")
                                        .disabled(stopped || docking)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.feed.start_dock(id, object_id);
                                            this.note = "Docking on the bucket".into();
                                            cx.notify();
                                        })),
                                ),
                            )
                        })
                })),
            )
        })
        .when_some(mark.dock.clone(), |tab, dock| {
            tab.child(dock_section(id, dock, cx))
        })
        .child(hint("More controls: Panels menu in the top bar.", cx))
}

/// Details panel: the trace, hardware, and navigation source and fix.
pub(super) fn details_panel(
    dashboard: &Dashboard,
    mark: &RoverMark,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
    let stopped = dashboard.is_stopped(mark);
    let (_, hardware_detail) = console::hardware_lines(mark, stopped);
    let source = console::source_index(mark);
    let mut navigation = Vec::new();
    if let Some(gps) = mark.raw_gps {
        navigation.push((
            "Position",
            format!(
                "{} · {:.5}, {:.5}{}{}",
                position_label(mark),
                gps.latitude,
                gps.longitude,
                gps.accuracy_m
                    .map(|accuracy| format!(" · ±{accuracy:.1} m"))
                    .unwrap_or_default(),
                gps.age_s
                    .map(|age| format!(" · {age:.1} s"))
                    .unwrap_or_default()
            ),
        ));
    } else {
        navigation.push(("Position", position_label(mark)));
    }
    if let Some(report) = mark.outdoor.as_ref() {
        navigation.push(("Heading", heading_line(report)));
        navigation.push(("Phase", phase_label(&report.phase)));
        if let Some(metres) = report.remaining_m {
            navigation.push(("Remaining", format!("{metres:.0} m")));
        }
        if let (Some(latitude), Some(longitude)) =
            (report.destination_latitude, report.destination_longitude)
        {
            navigation.push(("Destination", format!("{latitude:.5}, {longitude:.5}")));
        }
        navigation.push((
            "Arrival radius",
            format!("{:.0} m", report.arrival_radius_m),
        ));
    }
    v_flex()
        .gap_5()
        .child(rows(console::trace(mark, stopped)))
        // The trace already names the hardware state; only its detail adds.
        .when_some(hardware_detail, |tab, detail| {
            tab.child(section("Hardware", cx).child(caution(detail.to_owned(), cx)))
        })
        .child(
            section("Navigation", cx)
                .child(
                    ButtonGroup::new(("source", id))
                        .outline()
                        .small()
                        .children(console::NAVIGATION_SOURCES.iter().enumerate().map(
                            |(index, (_, label))| {
                                Button::new(("source", index as u64))
                                    .label(*label)
                                    .picked(source == index)
                            },
                        ))
                        .on_click(cx.listener(move |this, clicks: &Vec<usize>, _, cx| {
                            if this.selected == Some(id)
                                && let Some((wire, _)) = clicks
                                    .first()
                                    .and_then(|index| console::NAVIGATION_SOURCES.get(*index))
                            {
                                this.feed.select_source(id, wire);
                            }
                            cx.notify();
                        })),
                )
                .child(hint(console::navigation_note(mark), cx))
                .when(mark.app_update_required && mark.outdoor_gps, |section| {
                    section.child(caution(APP_UPDATE_REQUIRED, cx))
                })
                .when_some(
                    mark.outdoor.as_ref().and_then(|report| {
                        let label = report.operator_hold_label()?;
                        Some(if label == "Heading uncertain" {
                            format!("{label} · {}", heading_line(report))
                        } else {
                            label.to_owned()
                        })
                    }),
                    |section, label| section.child(caution(label, cx)),
                )
                .child(rows(navigation))
                .when(
                    mark.outdoor
                        .as_ref()
                        .is_some_and(|report| report.phase == "paused"),
                    |section| {
                        section.child(
                            h_flex().child(
                                Button::new("resume-outdoor")
                                    .outline()
                                    .small()
                                    .label("Resume outdoor mission")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.feed.resume_outdoor(id);
                                        cx.notify();
                                    })),
                            ),
                        )
                    },
                ),
        )
}

/// Study & tasks panel: study condition, L4 explore budget, L5 task box.
pub(super) fn autonomy_panel(
    dashboard: &Dashboard,
    mark: &RoverMark,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let stopped = dashboard.is_stopped(mark);
    v_flex()
        .gap_5()
        .child(study_section(dashboard, mark, cx))
        .child(explore_section(dashboard, mark, stopped, cx))
        .child(task_section(dashboard, mark, stopped, cx))
}

/// Goal & capture panel: typed goals and 3D point capture.
pub(super) fn goal_panel(
    dashboard: &Dashboard,
    mark: &RoverMark,
    facts: &Facts,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    v_flex()
        .gap_5()
        .child(goal_section(dashboard, mark, facts, cx))
        .child(capture_section(dashboard, mark.id, cx))
        .child(hint(
            "Click the map to place a destination, then Send goal in the popup. Outdoor GPS sends WGS84. Esc or Cancel sends nothing.",
            cx,
        ))
}
