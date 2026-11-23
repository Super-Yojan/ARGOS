//! Drive: the operator has taken one rover over and steers it by hand.
//!
//! Middle: the live view, with the speed readout and a small map. Right:
//! the drive link and the stop button ([`DriveControls`]). Left: the rest
//! of the fleet, so a waiting rover can still be answered mid-drive.
//!
//! The operator drives at one of two levels (`fleet::level`): L0 Manual, or
//! L1 Assisted with the obstacle assist. The switch is in [`DriveControls`].
//!
//! Taking over interrupts the goal the rover was driving to. Releasing
//! sends it back on its way.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::components::action_handler;
use crate::components::drive_controls::DriveControls;
use crate::components::fleet_map::FleetMap;
use crate::components::fpv_view::FpvView;
use crate::components::help_queue::HelpQueue;
use crate::components::rover_list::RoverList;
use crate::components::scene_view::SceneView;
use crate::components::widgets::{caps_label, mono, note, segment, segmented};
use crate::fleet::{Activity, Fleet, Level, Request, Rover, RoverAction, help_queue};
use crate::theme;

pub struct DrivePage {
    fleet: Fleet,
    controls: Entity<DriveControls>,
    /// The small map in the corner of the view.
    inset: Entity<FleetMap>,
    scene: Entity<SceneView>,
    fpv: Entity<FpvView>,
    show_fpv: bool,
    rover: Option<u64>,
    /// The goal the rover was driving to when it was taken over.
    interrupted: Option<InterruptedGoal>,
}

struct InterruptedGoal {
    task_id: u64,
    /// World frame, metres.
    goal: [f64; 2],
}

impl DrivePage {
    pub fn new(
        fleet: Fleet,
        scene: Entity<SceneView>,
        fpv: Entity<FpvView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let controls = cx.new(|cx| DriveControls::new(fleet.clone(), window, cx));
        let inset = cx.new(|_| FleetMap::inset(fleet.clone()));
        Self {
            fleet,
            controls,
            inset,
            scene,
            fpv,
            show_fpv: false,
            rover: None,
            interrupted: None,
        }
    }

    /// Start driving `rover_id` by hand, at `Level::Manual` or
    /// `Level::Assisted`.
    pub fn take_over(
        &mut self,
        rover_id: u64,
        level: Level,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.release(cx);

        // The rover ignores the stick while it drives to a goal, so put
        // the goal on hold and remember it for the release.
        let rover = self.fleet.rover(rover_id);
        self.interrupted = rover.as_ref().and_then(|rover| {
            let on_its_way = matches!(rover.activity, Activity::Driving { .. });
            Some(InterruptedGoal {
                task_id: rover.task_id?,
                goal: rover.goal?,
            })
            .filter(|_| on_its_way)
        });
        if self.interrupted.is_some() {
            self.fleet.perform(rover_id, RoverAction::Pause);
        }
        self.rover = Some(rover_id);

        let assist = level == Level::Assisted;
        self.controls.update(cx, |controls, cx| {
            controls.drive(rover_id, assist, window, cx)
        });
        self.inset
            .update(cx, |inset, cx| inset.follow(Some(rover_id), cx));
        self.fpv
            .update(cx, |fpv, cx| fpv.set_target_rover(Some(rover_id), cx));
        cx.notify();
    }

    /// Hand the rover back, and send it on to the goal it was taken from.
    pub fn release(&mut self, cx: &mut Context<Self>) {
        let Some(rover_id) = self.rover.take() else {
            return;
        };
        self.controls
            .update(cx, |controls, cx| controls.release(cx));

        if let Some(interrupted) = self.interrupted.take() {
            let still_its_task = self
                .fleet
                .rover(rover_id)
                .is_some_and(|rover| rover.task_id == Some(interrupted.task_id));
            if still_its_task {
                self.fleet.perform(rover_id, RoverAction::Resume);
            } else {
                // The obstacle assist replaced the task; send the goal again.
                let [x, y] = interrupted.goal;
                self.fleet.go_to(rover_id, x, y);
            }
        }
        cx.notify();
    }

    /// "You are driving Rover 4 · L1 Assisted", for the top bar.
    pub fn banner(&self, cx: &App) -> String {
        match self.rover {
            Some(rover_id) => {
                format!(
                    "You are driving Rover {rover_id} · {}",
                    self.controls.read(cx).level().label()
                )
            }
            None => "Nobody is being driven".into(),
        }
    }

    /// What the top bar's release button should say.
    pub fn release_label(&self) -> &'static str {
        if self.interrupted.is_some() {
            "Release and resume goal"
        } else {
            "Release"
        }
    }

    /// What an action on another rover means while driving.
    fn on_action(&mut self, request: &Request, window: &mut Window, cx: &mut Context<Self>) {
        match &request.action {
            RoverAction::TakeOver => self.take_over(request.rover, Level::Manual, window, cx),
            // There is no big map or focus panel on this page.
            RoverAction::Focus | RoverAction::SetGoal => {}
            command => self.fleet.perform(request.rover, command.clone()),
        }
        cx.notify();
    }

    fn rest_of_the_fleet(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let others: Vec<Rover> = self
            .fleet
            .rovers()
            .into_iter()
            .filter(|rover| Some(rover.id) != self.rover)
            .collect();
        let queue = help_queue(&others);
        let fine: Vec<Rover> = others
            .into_iter()
            .filter(|rover| rover.urgency().is_none())
            .collect();
        let on_action = action_handler(cx.listener(Self::on_action));

        v_flex()
            .id("drive-left")
            .w(px(240.0))
            .flex_none()
            .h_full()
            .gap_2()
            .overflow_y_scroll()
            .child(caps_label("Rest of the fleet"))
            .child(HelpQueue::new(queue, on_action.clone()).commands_only())
            .child(RoverList::new(fine, on_action))
            .child(note("Answering here does not stop your drive."))
    }

    fn live_view(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let twist = self.controls.read(cx).twist();
        let readout = |label: &'static str, value: String| {
            h_flex().gap_2().child(label).child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(theme::BLUE_PALE))
                    .child(value),
            )
        };
        let floating_box = || {
            div()
                .absolute()
                .bottom_3()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .overflow_hidden()
        };

        let viewport = if self.show_fpv {
            self.fpv.clone().into_any_element()
        } else {
            self.scene.clone().into_any_element()
        };

        let rover_id = self.rover;
        let gate_open = rover_id
            .and_then(|id| self.fleet.rover(id))
            .is_some_and(|r| r.gate_open);

        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .rounded(px(10.0))
            .border_2()
            .border_color(rgb(theme::BLUE))
            .overflow_hidden()
            .child(viewport)
            .child(
                h_flex()
                    .absolute()
                    .left(px(10.0))
                    .top(px(36.0))
                    .gap_2()
                    .occlude()
                    .child(
                        segmented()
                            .child(segment("drive-view-3d", "3D", !self.show_fpv).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.show_fpv = false;
                                    cx.notify();
                                }),
                            ))
                            .child(segment("drive-view-fpv", "FPV", self.show_fpv).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.show_fpv = true;
                                    if let Some(id) = this.rover {
                                        this.fpv
                                            .update(cx, |fpv, cx| fpv.set_target_rover(Some(id), cx));
                                    }
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        segment(
                            "drive-gate-toggle",
                            if gate_open { "Close gate" } else { "Open gate" },
                            gate_open,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(id) = this.rover {
                                this.fleet.set_gate(id, !gate_open);
                                cx.notify();
                            }
                        })),
                    ),
            )
            .child(
                floating_box()
                    .left_3()
                    .px_3()
                    .py_2()
                    .bg(rgb(theme::PANEL))
                    .child(
                        mono("")
                            .text_size(px(13.0))
                            .flex()
                            .gap_4()
                            .child(readout("speed", format!("{:+.2} m/s", twist.v)))
                            .child(readout("turn", format!("{:+.2} rad/s", twist.w)))
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .child("W A S D · release keys to stop"),
                            ),
                    ),
            )
            .child(
                floating_box()
                    .right_3()
                    .w(px(220.0))
                    .h(px(180.0))
                    .child(self.inset.clone()),
            )
    }
}

impl Render for DrivePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controls = self.controls.clone();
        let drive_keys = controls.read(cx).focus_handle();

        h_flex()
            .size_full()
            .items_start()
            .gap_3()
            // The whole page listens for the drive keys, so a click anywhere
            // on it gives the keyboard back to the rover.
            .track_focus(&drive_keys)
            .on_key_down(cx.listener({
                let controls = controls.clone();
                move |_, event, _, cx| {
                    controls.update(cx, |controls, cx| controls.key_down(event, cx))
                }
            }))
            .on_key_up(cx.listener({
                let controls = controls.clone();
                move |_, event, _, cx| {
                    controls.update(cx, |controls, cx| controls.key_up(event, cx))
                }
            }))
            .child(self.rest_of_the_fleet(cx))
            .child(self.live_view(cx))
            .child(
                div()
                    .id("drive-right")
                    .w(px(320.0))
                    .flex_none()
                    .h_full()
                    .overflow_y_scroll()
                    .child(controls),
            )
    }
}
