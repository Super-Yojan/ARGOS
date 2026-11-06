//! Top console bar. Always shown: title and fleet, 2D/3D, the Map menu
//! (imagery, grid), fleet control, link health, Panels, More (fleet, export), and
//! Stop all.

use super::palette;
use super::panels::PanelKind;
use super::video::stop_fpv_button;
use super::widgets::Picked;
use crate::Dashboard;

use argos_gpui::LaunchMode;
use argos_gpui::console::{self, StopOrigin};
use gpui_kit::base::h_flex;
use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt, TitleBar,
    button::{Button, ButtonGroup, ButtonVariants},
    menu::{DropdownMenu, PopupMenuItem},
    tooltip::Tooltip,
};
use gpui_kit::{prelude::*, *};
use terra_map::MapMode;

/// Window chrome for the console: map camera and grid,
/// this window's control claim, link health, and the fleet stop.
pub(super) fn console_bar(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let mode = dashboard.map_mode;
    let fleet = dashboard.fleet_mode;
    let held = dashboard.claim.held(dashboard.now);
    let detail: SharedString = format!("{} · {}", dashboard.link, dashboard.detail).into();
    // Video keeps running when its rover leaves the field; without a
    // selection there is no inspector to stop it from.
    let orphan_video = dashboard
        .fpv
        .view()
        .phone_id
        .filter(|_| dashboard.selected.is_none());
    let weak = cx.weak_entity();
    TitleBar::new()
        .child(
            h_flex()
                .gap_3()
                .flex_1()
                .min_w_0()
                // Window title with the fleet as its subtitle.
                .child(
                    h_flex()
                        .gap_2()
                        .items_baseline()
                        .child(div().text_sm().font_semibold().child("ARGOS"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(match fleet {
                                    LaunchMode::Sim => "Simulator",
                                    LaunchMode::Real => "Real rovers",
                                }),
                        ),
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
                .child({
                    let weak = weak.clone();
                    Button::new("map-menu")
                        .outline()
                        .small()
                        .label("Map ▾")
                        .tooltip("Map imagery and the occupancy grid")
                        .dropdown_menu(move |menu, _, cx| {
                            let (muted, grid) = weak
                                .upgrade()
                                .map(|this| {
                                    let this = this.read(cx);
                                    (this.muted_map, this.show_occupancy)
                                })
                                .unwrap_or((false, true));
                            let set_muted = |muted: bool| {
                                let weak = weak.clone();
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.muted_map = muted;
                                        this.map.set_survey(muted);
                                        cx.notify();
                                    });
                                }
                            };
                            let toggle_grid = {
                                let weak = weak.clone();
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.show_occupancy = !this.show_occupancy;
                                        cx.notify();
                                    });
                                }
                            };
                            menu.label("Imagery")
                                .item(
                                    PopupMenuItem::new("Colour")
                                        .checked(!muted)
                                        .on_click(set_muted(false)),
                                )
                                .item(
                                    PopupMenuItem::new("Muted")
                                        .checked(muted)
                                        .on_click(set_muted(true)),
                                )
                                .separator()
                                .item(
                                    PopupMenuItem::new("Occupancy grid")
                                        .checked(grid)
                                        .on_click(toggle_grid),
                                )
                        })
                }),
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
                            "Release fleet"
                        } else {
                            "Control fleet"
                        })
                        .tooltip(if held {
                            "Give up this window's control of the fleet (⌘⇧C)"
                        } else {
                            "Let this window send commands to the rovers (⌘⇧C)"
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_claim();
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                // Link as a coloured dot; words only when something is wrong.
                .child({
                    let fresh = dashboard.marks.iter().all(|mark| mark.reporting.is_fresh())
                        && dashboard.link == "healthy";
                    let label = console::link_label(&dashboard.link, &dashboard.marks);
                    h_flex()
                        .id("link-health")
                        .gap_1p5()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .tooltip(move |window, cx| Tooltip::new(detail.clone()).build(window, cx))
                        .child(div().size_2().rounded_full().bg(rgb(if fresh {
                            palette::SUCCESS
                        } else {
                            palette::DANGER
                        })))
                        .when(!fresh, |link| link.child(label))
                })
                .child({
                    let weak = weak.clone();
                    Button::new("panels-menu")
                        .outline()
                        .small()
                        .label("Panels ▾")
                        .tooltip("Open more rover controls as panels. Drag a tab to move it.")
                        .dropdown_menu(move |mut menu, _, cx| {
                            let open: Vec<bool> = weak
                                .upgrade()
                                .map(|this| {
                                    let this = this.read(cx);
                                    PanelKind::OPTIONAL
                                        .iter()
                                        .map(|kind| this.docks.is_open(*kind, cx))
                                        .collect()
                                })
                                .unwrap_or_default();
                            for (index, kind) in PanelKind::OPTIONAL.into_iter().enumerate() {
                                let weak = weak.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(kind.title())
                                        .checked(open.get(index).copied().unwrap_or(false))
                                        .on_click(move |_, window, cx| {
                                            let _ = weak.update(cx, |this, cx| {
                                                this.docks.toggle(kind, window, cx);
                                                cx.notify();
                                            });
                                        }),
                                );
                            }
                            menu
                        })
                })
                .child({
                    let weak = weak.clone();
                    Button::new("more-menu")
                        .ghost()
                        .small()
                        .label("More ▾")
                        .dropdown_menu(move |menu, _, _| {
                            let fleet_item = |label: &'static str, mode: LaunchMode| {
                                let weak = weak.clone();
                                PopupMenuItem::new(label).checked(fleet == mode).on_click(
                                    move |_, window, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.switch_fleet(mode, window);
                                            cx.notify();
                                        });
                                    },
                                )
                            };
                            let export = {
                                let weak = weak.clone();
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.export_session();
                                        cx.notify();
                                    });
                                }
                            };
                            menu.label("Fleet")
                                .item(fleet_item("Simulator (Zorvane)", LaunchMode::Sim))
                                .item(fleet_item("Real rovers", LaunchMode::Real))
                                .separator()
                                .item(PopupMenuItem::new("Export session log").on_click(export))
                        })
                })
                .child(
                    Button::new("stop-all")
                        .danger()
                        .small()
                        .label("Stop all")
                        .tooltip(
                            "Safety stop every rover (⌘.). Each needs Reset stop to move again.",
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.stop_all(StopOrigin::StopAll);
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                ),
        )
}
