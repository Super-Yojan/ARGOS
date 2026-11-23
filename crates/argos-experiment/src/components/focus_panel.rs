//! Everything about one rover: a live view, what happened, and what the
//! operator can do about it. Below it, a shortcut to the next rover waiting.

use std::rc::Rc;

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::help_queue::tone;
use super::level_picker::LevelPicker;
use super::widgets::{ButtonLook, caps_label, clock, heading, mono, note, panel, segment, segmented};
use super::{ActionHandler, action_button};
use crate::fleet::{Request, Rover, RoverAction};
use crate::theme;

#[derive(IntoElement)]
pub struct FocusPanel {
    rover: Option<Rover>,
    /// The next rover in the help queue, if any.
    next: Option<Rover>,
    /// True when the panel shows whoever is first in the queue, false when
    /// the operator picked this rover.
    following_queue: bool,
    viewport: AnyElement,
    show_fpv: bool,
    on_toggle_fpv: Rc<dyn Fn(bool, &mut Window, &mut App)>,
    on_action: ActionHandler,
}

impl FocusPanel {
    pub fn new(
        rover: Option<Rover>,
        viewport: AnyElement,
        show_fpv: bool,
        on_toggle_fpv: Rc<dyn Fn(bool, &mut Window, &mut App)>,
        on_action: ActionHandler,
    ) -> Self {
        Self {
            rover,
            next: None,
            following_queue: true,
            viewport,
            show_fpv,
            on_toggle_fpv,
            on_action,
        }
    }

    pub fn next_in_queue(mut self, next: Option<Rover>) -> Self {
        self.next = next;
        self
    }

    pub fn following_queue(mut self, following: bool) -> Self {
        self.following_queue = following;
        self
    }
}

impl RenderOnce for FocusPanel {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let column = v_flex().id("focus-panel");
        let Some(rover) = self.rover else {
            return column
                .child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(rgb(theme::BORDER_SUBTLE))
                        .child(heading("Focus")),
                )
                .child(div().p_3().child(note(
                    "No rover in sight yet. Rovers appear here as soon as they connect.",
                )));
        };

        let frame_color = rover
            .urgency()
            .map(|urgency| tone(urgency).accent)
            .unwrap_or(theme::BORDER);
        let follows = if self.following_queue {
            "queue top"
        } else {
            "selected"
        };

        let on_3d = self.on_toggle_fpv.clone();
        let on_fpv = self.on_toggle_fpv;
        column
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap_2()
                            .child(heading(rover.name()))
                            .child(
                                mono(follows)
                                    .text_size(px(10.5))
                                    .text_color(rgb(theme::TEXT_MUTED)),
                            ),
                    )
                    .child(
                        segmented()
                            .child(
                                segment("focus-view-3d", "3D", !self.show_fpv)
                                    .on_click(move |_, window, cx| on_3d(false, window, cx)),
                            )
                            .child(
                                segment("focus-view-fpv", "FPV", self.show_fpv)
                                    .on_click(move |_, window, cx| on_fpv(true, window, cx)),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .p(px(10.0))
                    .gap(px(10.0))
                    .child(
                        div()
                            .h(px(230.0))
                            .flex_none()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(frame_color))
                            .overflow_hidden()
                            .child(self.viewport),
                    )
                    .child(what_happened(&rover))
                    .child(LevelPicker::new(rover.clone(), self.on_action.clone()))
                    .child(what_you_can_do(&rover, &self.on_action))
                    .when_some(self.next, |column, next| {
                        column.child(next_in_queue(&next, &self.on_action))
                    }),
            )
    }
}

fn what_happened(rover: &Rover) -> Div {
    let position = match &rover.pose {
        Some(pose) if pose.tracked => format!("at {:.1}, {:.1} m", pose.x, pose.y),
        Some(_) => "position lost".into(),
        None => "no position yet".into(),
    };
    let gps_line = match (rover.geo_pose, rover.gps_anchor.as_ref()) {
        (Some(geo), Some(anchor)) => format!(
            "GPS {:.5}, {:.5} · hdg {:03.0}° · ±{:.1} m",
            geo.latitude, geo.longitude, geo.heading_deg, anchor.accuracy
        ),
        (None, Some(anchor)) => format!("GPS anchor ±{:.1} m (pose untracked)", anchor.accuracy),
        _ => "GPS: no anchor yet".into(),
    };
    let gps_goal_line = rover.gps_goal.map(|g| {
        format!(
            "GPS goal {:.5}, {:.5} (±{:.1} m)",
            g.latitude, g.longitude, g.tolerance
        )
    });
    panel()
        .gap(px(6.0))
        .child(caps_label("What happened"))
        .child(div().text_size(px(15.0)).child(format!(
            "{}. {}",
            rover.headline(),
            rover.explanation()
        )))
        .child(mono(position).text_color(rgb(theme::TEXT_MUTED)))
        .child(mono(gps_line).text_size(px(11.0)).text_color(rgb(theme::TEXT_MUTED)))
        .when_some(gps_goal_line, |panel, line| {
            panel.child(mono(line).text_size(px(11.0)).text_color(rgb(theme::BLUE_TEXT)))
        })
        .when_some(rover.rejected.clone(), |panel, reason| {
            panel.child(
                mono(format!("Last request refused: {reason}")).text_color(rgb(theme::RED.fill)),
            )
        })
}

fn what_you_can_do(rover: &Rover, on_action: &ActionHandler) -> Div {
    // One slot shows whichever of the pair applies right now.
    let pause_or_resume = if rover.allows(&RoverAction::Pause) {
        RoverAction::Pause
    } else {
        RoverAction::Resume
    };
    let stop_or_release = if rover.stopped_here {
        RoverAction::ReleaseStop
    } else {
        RoverAction::Stop
    };
    let stop_look = if rover.stopped_here {
        ButtonLook::Outline
    } else {
        ButtonLook::Stop
    };
    let stop_label = if rover.stopped_here {
        RoverAction::ReleaseStop.label().to_string()
    } else {
        format!("Stop {}", rover.name())
    };

    let pair = || h_flex().gap_2();
    let outline = |action: RoverAction| {
        let label = action.label();
        action_button(rover, action, label, ButtonLook::Outline, on_action).flex_1()
    };

    let gate_action = RoverAction::SetGate(!rover.gate_open);
    let gate_label = gate_action.label();
    let gate_look = if rover.gate_open {
        ButtonLook::Primary
    } else {
        ButtonLook::Outline
    };

    panel()
        .child(caps_label("What you can do"))
        .child(
            action_button(
                rover,
                RoverAction::TakeOver,
                "Take over and drive",
                ButtonLook::Primary,
                on_action,
            )
            .min_h(px(52.0))
            .text_size(px(15.0)),
        )
        .child(
            pair()
                .child(outline(pause_or_resume))
                .child(outline(RoverAction::SetGoal)),
        )
        .child(
            pair()
                .child(action_button(rover, gate_action, gate_label, gate_look, on_action).flex_1())
                .child(outline(RoverAction::Clear)),
        )
        .child(action_button(
            rover,
            stop_or_release,
            stop_label,
            stop_look,
            on_action,
        ))
}

fn next_in_queue(next: &Rover, on_action: &ActionHandler) -> Div {
    let on_action = on_action.clone();
    let focus = Request {
        rover: next.id,
        action: RoverAction::Focus,
    };
    let accent = next
        .urgency()
        .map(|urgency| tone(urgency).accent)
        .unwrap_or(theme::BORDER);

    v_flex()
        .gap(px(6.0))
        .child(caps_label("Next in queue"))
        .child(
            h_flex()
                .id("next-in-queue")
                .min_h(px(56.0))
                .px(px(10.0))
                .gap_3()
                .items_center()
                .justify_between()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(accent))
                .bg(rgb(theme::PANEL))
                .cursor_pointer()
                .on_click(move |_, window, cx| on_action(&focus, window, cx))
                .child(
                    v_flex()
                        .child(div().font_weight(FontWeight::BOLD).child(next.name()))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(theme::ORANGE_TEXT))
                                .child(next.headline()),
                        ),
                )
                .child(mono(clock(next.waiting)).text_size(px(14.0))),
        )
}
