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
use super::video::video_tab;
use super::widgets::caution;
use super::widgets::hint;
use super::widgets::rows;
use super::widgets::section;
use crate::Dashboard;

use argos_core::{
    APP_UPDATE_REQUIRED, DragGate, HEADING_MAX_AGE_S, HEADING_MAX_UNCERTAINTY_DEG, OPERATOR_LEVELS,
    OutdoorReport, drag_disabled_reason, drive_block, phase_label,
};
use argos_gpui::console::{self, InspectorTab, StopOrigin};
use argos_gpui::{RoverMark, position_label};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt, ThemeStyled,
    button::{Button, ButtonGroup, ButtonVariants},
    scroll::ScrollableElement,
    tab::{Tab, TabBar},
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
    let tab = dashboard.inspector_tab;
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
    let body = match tab {
        InspectorTab::Overview => overview_tab(dashboard, mark, &facts, cx).into_any_element(),
        InspectorTab::Mission => {
            super::mission::mission_tab(dashboard, mark, cx).into_any_element()
        }
        InspectorTab::Video => video_tab(dashboard, cx).into_any_element(),
    };
    v_flex()
        .absolute()
        .top_3()
        .right_3()
        .bottom_3()
        .w(px(360.))
        .popover_style(cx)
        .overflow_hidden()
        // Measured so rover cards can stay out from under the inspector.
        .child({
            let frame = dashboard.inspector_frame.clone();
            canvas(
                move |bounds, _, _| *frame.lock().unwrap() = Some(bounds),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            v_flex()
                .flex_none()
                .p_4()
                .pb_2()
                .gap_3()
                // Stop sits top right, on the title row: away from Take over
                // and two rows above the tab strip, so a reach for the tabs
                // cannot land on it. Still one click, plus Space anywhere.
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_semibold()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .child(format!("Rover {id}")),
                                )
                                .when(reversing, |tags| {
                                    tags.child(Tag::warning().small().child("Reversing"))
                                })
                                .children(callout.tag.map(callout_tag)),
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
                .child(div().text_base().font_medium().child(callout.state))
                // Above the tabs, so the way out shows on Overview and Video alike.
                .when_some(callout.recovery, |header, recovery| {
                    header.child(caution(recovery, cx))
                })
                .child(h_flex().gap_2().child(primary.outline().small()))
                .child(div().h_2()),
        )
        .child(
            TabBar::new("inspector-tabs")
                .underline()
                .small()
                .px_2()
                .selected_index(tab.index())
                .children(
                    InspectorTab::ALL
                        .into_iter()
                        .map(|tab| Tab::new().label(tab.label())),
                )
                .on_click(cx.listener(|this, index: &usize, _, cx| {
                    this.inspector_tab = InspectorTab::from_index(*index);
                    cx.notify();
                })),
        )
        .child(
            v_flex()
                .id("inspector-body")
                // Zero basis: the body takes only the height left under the
                // header and tab bar, and scrolls the rest.
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

fn overview_tab(
    dashboard: &Dashboard,
    mark: &RoverMark,
    facts: &Facts,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
    let stopped = dashboard.is_stopped(mark);
    let driving = dashboard.driving;
    let (hardware_status, hardware_detail) = console::hardware_lines(mark, stopped);
    let level = console::level_index(mark);
    let lock = dashboard.study_lock(mark).map(str::to_owned);
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
        // Found buckets and the dock sit up top: Dock is the next step once one is found.
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
        .when_some(mark.dock.clone(), |tab, dock| tab.child(dock_section(id, dock, cx)))
        .child(
            section("Hardware", cx)
                .child(div().text_sm().child(hardware_status.to_owned()))
                .when_some(hardware_detail, |section, detail| {
                    section.child(caution(detail.to_owned(), cx))
                })
                // Reset stop clears this window's latch or the rover's own stop.
                .when(stopped, |section| {
                    section.child(
                        h_flex().child(
                            Button::new(("reset-stop", id))
                                .outline()
                                .small()
                                .label("Reset stop")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.reset_safety(id);
                                    cx.notify();
                                })),
                        ),
                    )
                }),
        )
        .child(
            section("Autonomy", cx)
                .child(hint(console::level_line(mark), cx))
                // L0–L3, then L4 and L5, so neither row wraps.
                .child(level_group(id, level, lock.clone(), 0..4, cx))
                .child(level_group(id, level, lock, 4..OPERATOR_LEVELS.len(), cx)),
        )
        .child(study_section(dashboard, mark, cx))
        .child(explore_section(dashboard, mark, stopped, cx))
        .child(task_section(dashboard, mark, stopped, cx))
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
                                    .selected(source == index)
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
        .child(goal_section(dashboard, mark, facts, cx))
        .child(capture_section(dashboard, id, cx))
        .when(driving, |tab| tab.child(drive_section(dashboard, mark, facts.limits, cx)))
        .child(hint(
            "Click the map to place a destination, then Send goal in the popup. The rover stays at its live position. Outdoor GPS sends WGS84. Esc or Cancel sends nothing. Drive sends commands while keys are held. Stop halts the rover; Reset stop clears the stop.",
            cx,
        ))
}
