//! Top console bar: map mode, grid toggle, status, control claim, link
//! health, fleet switch, and Stop all.

use super::video::stop_fpv_button;
use super::widgets::Picked;
use crate::Dashboard;

use argos_gpui::LaunchMode;
use argos_gpui::console::{self, StopOrigin};
use gpui_kit::base::h_flex;
use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt, TitleBar,
    button::{Button, ButtonGroup, ButtonVariants},
    tooltip::Tooltip,
};
use gpui_kit::{prelude::*, *};
use terra_map::MapMode;

/// Window chrome for the console: map camera and grid, the latest status,
/// this window's control claim, link health, and the fleet stop.
pub(super) fn console_bar(
    dashboard: &Dashboard,
    status: String,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let mode = dashboard.map_mode;
    let fleet = dashboard.fleet_mode;
    let held = dashboard.claim.held(dashboard.now);
    let detail: SharedString = format!("{} · {}", dashboard.link, dashboard.detail).into();
    let status: SharedString = status.into();
    // Video keeps running when its rover leaves the field; without a
    // selection there is no inspector to stop it from.
    let orphan_video = dashboard
        .fpv
        .view()
        .phone_id
        .filter(|_| dashboard.selected.is_none());
    TitleBar::new()
        .child(
            h_flex()
                .gap_3()
                .flex_1()
                .min_w_0()
                .child(div().text_sm().font_semibold().child("ARGOS"))
                .child(
                    ButtonGroup::new("fleet-mode")
                        .outline()
                        .small()
                        .child(
                            Button::new("fleet-sim")
                                .label("Sim")
                                .picked(fleet == LaunchMode::Sim)
                                .tooltip("Local Zorvane simulator at tcp/127.0.0.1:7447"),
                        )
                        .child(
                            Button::new("fleet-real")
                                .label("Real")
                                .picked(fleet == LaunchMode::Real)
                                .tooltip("Real fleet from the environment and .env"),
                        )
                        .on_click(cx.listener(|this, clicks: &Vec<usize>, window, cx| {
                            this.switch_fleet(
                                if clicks.contains(&0) {
                                    LaunchMode::Sim
                                } else {
                                    LaunchMode::Real
                                },
                                window,
                            );
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    ButtonGroup::new("map-mode")
                        .outline()
                        .small()
                        .child(
                            Button::new("map-2d")
                                .label("2D")
                                .picked(mode == MapMode::TwoD),
                        )
                        .child(
                            Button::new("map-3d")
                                .label("3D")
                                .picked(mode == MapMode::ThreeD),
                        )
                        .on_click(cx.listener(|this, clicks: &Vec<usize>, window, cx| {
                            this.set_map_mode(if clicks.contains(&0) {
                                MapMode::TwoD
                            } else {
                                MapMode::ThreeD
                            });
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    ButtonGroup::new("map-style")
                        .outline()
                        .small()
                        .child(
                            Button::new("map-colour")
                                .label("Colour")
                                .picked(!dashboard.muted_map)
                                .tooltip("True-colour satellite imagery"),
                        )
                        .child(
                            Button::new("map-muted")
                                .label("Muted")
                                .picked(dashboard.muted_map)
                                .tooltip("Muted blue-grey imagery so overlays stand out"),
                        )
                        .on_click(cx.listener(|this, clicks: &Vec<usize>, window, cx| {
                            this.muted_map = !clicks.contains(&0);
                            this.map.set_survey(this.muted_map);
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("occupancy-overlay")
                        .outline()
                        .small()
                        .picked(dashboard.show_occupancy)
                        .label("Map grid")
                        .tooltip("Show the selected rover's occupancy grid on the map")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_occupancy = !this.show_occupancy;
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("console-status")
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .when(!status.is_empty(), |line| {
                            let full = status.clone();
                            line.tooltip(move |window, cx| {
                                Tooltip::new(full.clone()).build(window, cx)
                            })
                        })
                        .child(status),
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .pr_2()
                .flex_none()
                .when_some(orphan_video, |bar, active| {
                    bar.child(
                        stop_fpv_button(cx)
                            .ghost()
                            .tooltip(format!("Video is rover {active}")),
                    )
                })
                .child(
                    Button::new("control-claim")
                        .outline()
                        .small()
                        .picked(held)
                        .label(if held {
                            "Release control"
                        } else {
                            "Take control"
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.claim.held(this.now) {
                                this.claim.release();
                                // Releasing control also closes the rover panel.
                                this.stop_drive();
                                this.drag = None;
                                this.selected = None;
                                this.selection_hold.clear();
                                this.note = "Control released".into();
                            } else {
                                this.claim.take(this.now);
                                this.note = "Control held by this window".into();
                            }
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("link-health")
                        .text_xs()
                        .text_color(
                            if dashboard.marks.iter().all(|mark| mark.reporting.is_fresh()) {
                                cx.theme().muted_foreground
                            } else {
                                cx.theme().danger
                            },
                        )
                        .tooltip(move |window, cx| Tooltip::new(detail.clone()).build(window, cx))
                        .child(console::link_label(&dashboard.link, &dashboard.marks)),
                )
                .child(
                    Button::new("export-session")
                        .outline()
                        .small()
                        .label("Export log")
                        .tooltip(format!(
                            "Save a copy of this window's session log in {}. The live log is kept there too.",
                            argos_gpui::session_dir().display()
                        ))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.export_session();
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("stop-all")
                        .danger()
                        .small()
                        .label("Stop all")
                        .tooltip("Safety stop every rover. Each needs Reset stop to move again.")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.stop_all(StopOrigin::StopAll);
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                ),
        )
}
