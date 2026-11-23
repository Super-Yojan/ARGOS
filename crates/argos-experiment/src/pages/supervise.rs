//! Supervise: watch the whole fleet and deal with whoever needs you.
//!
//! Left: the help queue, then the rovers that are fine. Middle: the map.
//! Right: the rover in focus, which is the first in the queue unless the
//! operator picked another one.

use std::rc::Rc;

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::components::action_handler;
use crate::components::fleet_map::{FleetMap, MapEvent};
use crate::components::focus_panel::FocusPanel;
use crate::components::fpv_view::FpvView;
use crate::components::geo_map::{GeoMap, GeoMapEvent};
use crate::components::help_queue::HelpQueue;
use crate::components::rover_list::RoverList;
use crate::components::scene_view::SceneView;
use crate::components::widgets::{caps_label, heading, mono, note, segment, segmented};
use crate::fleet::{Fleet, Level, Request, Rover, RoverAction, help_queue};
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum MapKind {
    Local,
    Geo,
}

pub enum SuperviseEvent {
    /// The operator wants to drive this rover by hand, at a manual level
    /// (L0, or L1 with the obstacle assist).
    TakeOver { rover: u64, level: Level },
}

pub struct SupervisePage {
    fleet: Fleet,
    map: Entity<FleetMap>,
    geo_map: Entity<GeoMap>,
    map_kind: MapKind,
    scene: Entity<SceneView>,
    fpv: Entity<FpvView>,
    show_fpv: bool,
    /// The rover the operator chose to look at, if they chose one.
    picked: Option<Pick>,
    _subscriptions: Vec<Subscription>,
}

/// A pick lasts until a different rover reaches the top of the help queue:
/// then the focus goes back to following the queue.
struct Pick {
    rover: u64,
    queue_top_then: Option<u64>,
}

impl EventEmitter<SuperviseEvent> for SupervisePage {}

impl SupervisePage {
    pub fn new(
        fleet: Fleet,
        scene: Entity<SceneView>,
        fpv: Entity<FpvView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let map = cx.new(|_| FleetMap::new(fleet.clone()));
        let geo_map = cx.new(|cx| GeoMap::new(fleet.clone(), cx));
        let subscriptions = vec![
            cx.subscribe(&map, |this, _, event, cx| match event {
                MapEvent::RoverPicked(rover_id) => this.pick(*rover_id, cx),
            }),
            cx.subscribe(&geo_map, |this, _, event, cx| match event {
                GeoMapEvent::RoverPicked(rover_id) => this.pick(*rover_id, cx),
            }),
        ];
        Self {
            fleet,
            map,
            geo_map,
            map_kind: MapKind::Local,
            scene,
            fpv,
            show_fpv: false,
            picked: None,
            _subscriptions: subscriptions,
        }
    }

    fn set_show_fpv(&mut self, show_fpv: bool, rover_id: Option<u64>, cx: &mut Context<Self>) {
        self.show_fpv = show_fpv;
        self.fpv
            .update(cx, |fpv, cx| fpv.set_target_rover(rover_id, cx));
        cx.notify();
    }

    fn pick(&mut self, rover_id: u64, cx: &mut Context<Self>) {
        let queue_top = help_queue(&self.fleet.rovers())
            .first()
            .map(|rover| rover.id);
        self.picked = Some(Pick {
            rover: rover_id,
            queue_top_then: queue_top,
        });
        if self.map_kind == MapKind::Geo
            && let Some(geo) = self.fleet.rover(rover_id).and_then(|r| r.geo_pose)
        {
            self.geo_map.read(cx).fly_to(geo.latitude, geo.longitude);
        }
        cx.notify();
    }

    /// What an action means on this page.
    fn on_action(&mut self, request: &Request, _: &mut Window, cx: &mut Context<Self>) {
        let rover_id = request.rover;
        match &request.action {
            RoverAction::Focus => self.pick(rover_id, cx),
            RoverAction::TakeOver => cx.emit(SuperviseEvent::TakeOver {
                rover: rover_id,
                level: Level::Manual,
            }),
            // A manual level is driven from the drive view.
            RoverAction::SetLevel(level) if level.is_manual() => {
                cx.emit(SuperviseEvent::TakeOver {
                    rover: rover_id,
                    level: *level,
                })
            }
            RoverAction::SetGoal => {
                self.pick(rover_id, cx);
                match self.map_kind {
                    MapKind::Local => self
                        .map
                        .update(cx, |map, cx| map.pick_goal_for(rover_id, cx)),
                    MapKind::Geo => self
                        .geo_map
                        .update(cx, |map, cx| map.pick_goal_for(rover_id, cx)),
                }
            }
            command => self.fleet.perform(rover_id, command.clone()),
        }
        cx.notify();
    }

    /// The rover the focus panel shows, and whether it follows the queue.
    fn focus(&mut self, rovers: &[Rover], queue: &[Rover]) -> (Option<Rover>, bool) {
        let queue_top = queue.first().map(|rover| rover.id);
        let picked = self
            .picked
            .as_ref()
            .filter(|pick| pick.queue_top_then == queue_top || queue_top == Some(pick.rover))
            .and_then(|pick| rovers.iter().find(|rover| rover.id == pick.rover));
        match picked {
            Some(rover) => (Some(rover.clone()), false),
            None => {
                self.picked = None;
                (queue.first().or(rovers.first()).cloned(), true)
            }
        }
    }
}

impl Render for SupervisePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rovers = self.fleet.rovers();
        let queue = help_queue(&rovers);
        let on_their_own: Vec<Rover> = rovers
            .iter()
            .filter(|rover| rover.urgency().is_none())
            .cloned()
            .collect();
        let (focused, following_queue) = self.focus(&rovers, &queue);
        let focused_id = focused.as_ref().map(|rover| rover.id);
        let next = queue
            .iter()
            .find(|rover| Some(rover.id) != focused_id)
            .cloned();

        let on_action = action_handler(cx.listener(Self::on_action));

        let needs_you = v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .items_baseline()
                    .child(caps_label(&format!("Needs you ({})", queue.len())))
                    .child(
                        mono("urgent first")
                            .text_size(px(10.5))
                            .text_color(rgb(theme::TEXT_MUTED)),
                    ),
            )
            .child(if queue.is_empty() {
                note("All clear — nobody waiting.").into_any_element()
            } else {
                HelpQueue::new(queue, on_action.clone()).into_any_element()
            });

        let fine = v_flex()
            .gap(px(6.0))
            .child(caps_label(&format!("Online ({})", on_their_own.len())))
            .child(RoverList::new(on_their_own, on_action.clone()).selected(focused_id));

        let left = v_flex()
            .w(px(300.0))
            .flex_none()
            .h_full()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .bg(rgb(theme::PANEL))
            .overflow_hidden()
            .child(
                h_flex()
                    .flex_none()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(heading("Rovers"))
                    .child(
                        mono(format!("{} total", rovers.len()))
                            .text_size(px(11.0))
                            .text_color(rgb(theme::TEXT_MUTED)),
                    ),
            )
            .child(
                v_flex()
                    .id("supervise-left")
                    .flex_1()
                    .min_h_0()
                    .p(px(10.0))
                    .gap_3()
                    .overflow_y_scroll()
                    .child(needs_you)
                    .child(div().h(px(1.0)).w_full().bg(rgb(theme::BORDER_SUBTLE)))
                    .child(fine),
            );

        self.geo_map
            .update(cx, |geo, _| geo.set_focused_rover(focused_id));

        let active_map = match self.map_kind {
            MapKind::Local => self.map.clone().into_any_element(),
            MapKind::Geo => self.geo_map.clone().into_any_element(),
        };

        let middle = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .bg(rgb(theme::PANEL))
            .overflow_hidden()
            .child(
                h_flex()
                    .flex_none()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(heading("Map"))
                            .child(
                                segmented()
                                    .child(
                                        segment("map-kind-local", "Local", self.map_kind == MapKind::Local)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.map_kind = MapKind::Local;
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        segment("map-kind-geo", "Geo", self.map_kind == MapKind::Geo)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.map_kind = MapKind::Geo;
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        mono(format!(
                            "{} GPS",
                            rovers.iter().filter(|r| r.geo_pose.is_some()).count()
                        ))
                        .text_size(px(11.0))
                        .text_color(rgb(theme::TEXT_MUTED)),
                    ),
            )
            .child(div().flex_1().min_h_0().w_full().child(active_map));

        self.fpv
            .update(cx, |fpv, cx| fpv.set_target_rover(focused_id, cx));
        let viewport = if self.show_fpv {
            self.fpv.clone().into_any_element()
        } else {
            self.scene.clone().into_any_element()
        };
        let on_toggle_fpv: Rc<dyn Fn(bool, &mut Window, &mut App)> = {
            let listener = cx.listener(move |this, show_fpv: &bool, _, cx| {
                this.set_show_fpv(*show_fpv, focused_id, cx);
            });
            Rc::new(move |show_fpv, window, cx| listener(&show_fpv, window, cx))
        };

        let right = div()
            .id("supervise-right")
            .w(px(370.0))
            .flex_none()
            .h_full()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .bg(rgb(theme::PANEL))
            .overflow_y_scroll()
            .child(
                FocusPanel::new(focused, viewport, self.show_fpv, on_toggle_fpv, on_action)
                    .next_in_queue(next)
                    .following_queue(following_queue),
            );

        h_flex()
            .size_full()
            .items_start()
            .gap_2()
            .child(left)
            .child(middle)
            .child(right)
    }
}
