//! Smaller Overview sections: goal, bucket dock, and 3D point capture.

use super::inspector::Facts;
use super::inspector::goal_line;
use super::widgets::hint;
use super::widgets::rows;
use super::widgets::section;
use crate::Dashboard;

use argos_gpui::RoverMark;
use argos_gpui::console::{self};
use gpui_kit::base::h_flex;
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{prelude::*, *};

pub(super) fn goal_section(
    dashboard: &Dashboard,
    mark: &RoverMark,
    facts: &Facts,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
    section("Goal", cx)
        .child(
            h_flex()
                .gap_2()
                .justify_between()
                .child(div().min_w_0().flex_1().text_xs().child(goal_line(
                    mark,
                    facts.drag,
                    &dashboard.link,
                )))
                .child(
                    Button::new("cancel-goal")
                        .outline()
                        .small()
                        .label("Cancel goal")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.feed.cancel_goal(id);
                            this.drag = None;
                            this.note = "Cancel requested".into();
                            cx.notify();
                        })),
                ),
        )
        .when(console::path_provisional(mark), |section| {
            section.child(hint(
                "Path direction is provisional until a segment velocity is published.",
                cx,
            ))
        })
}

/// The proposed destination's popup, anchored just above the clicked point.
/// Bucket dock status with Dump and Cancel. The dump pop-up offers the same Dump.
pub(super) fn dock_section(
    id: u64,
    dock: argos_core::DockReport,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let active = dock.active();
    let awaiting = dock.awaiting_approval;
    let dock_id = dock.dock_id;
    section("Bucket dock", cx)
        .child(rows(dock.rows()))
        .when(active, |section| {
            section.child(
                h_flex()
                    .gap_2()
                    .when(awaiting, |row| {
                        row.child(
                            Button::new(("dock-dump", id))
                                .primary()
                                .small()
                                .label("Dump")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.feed.approve_dump(id, dock_id);
                                    this.note = "Dump approved".into();
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new(("dock-cancel", id))
                            .outline()
                            .small()
                            .label("Cancel dock")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.feed.cancel_dock(id);
                                this.note = "Dock cancel requested".into();
                                cx.notify();
                            })),
                    ),
            )
        })
}

/// Capture 3D: a full-resolution point cloud labelled by the rover's mapper,
/// saved as a colour-coded PLY file to debug false obstacles.
pub(super) fn capture_section(
    dashboard: &Dashboard,
    id: u64,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let status = dashboard.feed.point_capture(id);
    let running = status.as_ref().is_some_and(|s| !s.finished);
    let lines = status.as_ref().map(argos_gpui::capture_lines);
    let live = dashboard.feed.live_points(id);
    let live_on = live.as_ref().is_some_and(|l| l.ended.is_none());
    section("3D capture", cx)
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(("live-3d", id))
                        .outline()
                        .small()
                        .selected(live_on)
                        .label("Live 3D")
                        .tooltip(
                            "Streams the rover's depth labelled by its map rules and draws it on the map under the rover. Turn off to stop the stream",
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.selected == Some(id) {
                                let on = this
                                    .feed
                                    .live_points(id)
                                    .is_some_and(|l| l.ended.is_none());
                                this.feed.set_live_points(id, !on);
                            }
                            cx.notify();
                        })),
                )
                .child(
                Button::new(("capture-3d", id))
                    .outline()
                    .small()
                    .label(if running { "Capturing…" } else { "Capture 3D" })
                    .tooltip(
                        "Records every depth pixel for 10 s at 1 Hz, labelled the way the rover's map classifies it, and saves a PLY file",
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let busy = this.feed.point_capture(id).is_some_and(|s| !s.finished);
                        if this.selected == Some(id) && !busy {
                            this.feed.capture_3d(id);
                        }
                        cx.notify();
                    })),
            ),
        )
        .when_some(live.as_ref().map(argos_gpui::live_line), |section, line| {
            section.child(div().text_sm().child(line))
        })
        .map(|section| match lines {
            Some((progress, counts)) => section
                .child(div().text_sm().child(progress))
                .when_some(counts, |section, counts| section.child(hint(counts, cx))),
            None => section.child(hint(
                format!(
                    "Colours: free green, obstacle red, overhead blue, out of range grey, outside band yellow. Files go to {}.",
                    argos_gpui::capture_dir().display()
                ),
                cx,
            )),
        })
}
