//! Mission tab: the operator plans a list of steps on the map (waypoint,
//! collect, home/refill, score at a found bucket) and ARGOS works through it
//! with the goals and docks Terra already takes. The state machine is
//! `argos_core::MissionRun`; this file keeps one per rover, ticks it every
//! sync, sends what it asks for, and draws the tab.

use super::widgets::Picked;
use super::widgets::{caution, hint, section};
use crate::Dashboard;

use argos_core::{
    GoalRequest, MissionCommand, MissionPlan, MissionRun, MissionStep, MissionTick, MissionView,
    Phase, StepKind, Wait, mission_block,
};
use argos_gpui::RoverMark;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonGroup, ButtonVariants},
};
use gpui_kit::{prelude::*, *};
use std::collections::BTreeMap;

/// Every rover's plan and running mission, and what a map click adds.
#[derive(Default)]
pub(crate) struct MissionBoard {
    pub(crate) plans: BTreeMap<u64, MissionPlan>,
    pub(crate) runs: BTreeMap<u64, MissionRun>,
    /// While set, a map click adds a step of this kind to the selected rover's plan.
    pub(crate) placing: Option<StepKind>,
}

impl MissionBoard {
    /// The steps to draw for a rover: the running mission's, else the plan.
    pub(crate) fn shown(&self, id: u64) -> Option<(&[MissionStep], Option<usize>)> {
        if let Some(run) = self.runs.get(&id).filter(|run| run.active()) {
            return Some((&run.plan.steps, Some(run.index)));
        }
        self.plans
            .get(&id)
            .filter(|plan| !plan.steps.is_empty())
            .map(|plan| (plan.steps.as_slice(), None))
    }
}

/// Live bucket positions a rover has found: (id, x, y) in its local frame.
fn buckets(mark: &RoverMark) -> Vec<(u32, f64, f64)> {
    mark.objects
        .iter()
        .filter(|object| object.object.class == "bucket")
        .map(|object| (object.object.id, object.object.x, object.object.y))
        .collect()
}

/// "Orange bucket 4".
fn bucket_label(object: &argos_core::FoundObject) -> String {
    let label = object.label();
    let mut chars = label.chars();
    let first = chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    format!("{first}{} {}", chars.as_str(), object.id)
}

fn halted(mark: &RoverMark) -> Option<&'static str> {
    if mark.safety == "emergency_stop" {
        Some("Rover is stopped · reset the stop, then Resume")
    } else if mark.status != "online" {
        Some("Rover pose is not fresh")
    } else if mark.outdoor_gps {
        Some("Missions use local goals · switch the source off Outdoor GPS")
    } else {
        None
    }
}

/// Runs `with` on the mission view of `mark`.
fn with_view<R>(mark: &RoverMark, with: impl FnOnce(&MissionView) -> R) -> R {
    let buckets = buckets(mark);
    let level = mark
        .effective_level
        .as_deref()
        .or(mark.requested_level.as_deref());
    let view = MissionView {
        x: mark.local_x,
        y: mark.local_y,
        level,
        goal: mark
            .goal
            .as_ref()
            .map(|goal| (goal.x, goal.y, goal.state.as_str())),
        dock: mark.dock.as_ref(),
        buckets: &buckets,
        halted: halted(mark),
    };
    with(&view)
}

impl Dashboard {
    /// Tick every running mission with the newest fleet picture.
    pub(super) fn sync_missions(&mut self) {
        let ids: Vec<u64> = self.missions.runs.keys().copied().collect();
        for id in ids {
            let mark = self.marks.iter().find(|mark| mark.id == id).cloned();
            let now = self.now;
            let Some(run) = self.missions.runs.get_mut(&id) else {
                continue;
            };
            let tick = match &mark {
                Some(mark) => with_view(mark, |view| run.tick(view, now)),
                None => run.pause_now("Rover left the fleet"),
            };
            self.apply_mission(id, tick);
        }
    }

    fn apply_mission(&mut self, id: u64, tick: MissionTick) {
        for command in tick.commands {
            match command {
                MissionCommand::Goal { x, y, yaw } => {
                    self.feed
                        .publish_goal(id, GoalRequest::Local { x, y, yaw }, None)
                }
                MissionCommand::Dock { bucket } => self.feed.start_dock(id, bucket),
                MissionCommand::CancelGoal => self.feed.cancel_goal(id),
                MissionCommand::CancelDock => self.feed.cancel_dock(id),
            }
        }
        for event in tick.events {
            if event.event == "paused"
                && let Some(reason) = &event.detail
            {
                self.note = format!("Rover {id} mission paused: {reason}");
            }
            self.feed.record(serde_json::json!({
                "kind": "mission",
                "rover_id": id,
                "event": event.event,
                "step": event.index + 1,
                "step_kind": event.kind.name(),
                "detail": event.detail,
            }));
        }
    }

    /// A map click while placing: add a step at the clicked ground point.
    /// Returns false when not placing, so the click proposes a goal as usual.
    pub(super) fn place_mission_step(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let (Some(kind), Some(id)) = (self.missions.placing, self.selected) else {
            return false;
        };
        let Some(frame) = self.selected_mark().and_then(|mark| mark.overlay_frame) else {
            self.note = "No local position yet · the rover needs a map frame".into();
            return true;
        };
        let Some((latitude, longitude)) = self.view.read(cx).ground_at(position) else {
            self.note = "No ground under that point".into();
            return true;
        };
        let Ok((x, y)) = frame.local(latitude, longitude) else {
            self.note = "That point is outside the rover's map frame".into();
            return true;
        };
        let plan = self.missions.plans.entry(id).or_default();
        self.note = match plan.push(MissionStep::at(kind, x, y)) {
            Ok(()) => format!(
                "Step {} added: {} ({x:.1}, {y:.1})",
                plan.steps.len(),
                kind.label()
            ),
            Err(reason) => reason.into(),
        };
        true
    }

    fn start_mission(&mut self, id: u64) {
        if !self.claim.held(self.now) {
            self.note = "Control the fleet to start a mission".into();
            return;
        }
        if self.driving {
            self.note = "Leave drive before starting a mission".into();
            return;
        }
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id).cloned() else {
            return;
        };
        let plan = self.missions.plans.get(&id).cloned().unwrap_or_default();
        let now = self.now;
        match with_view(&mark, |view| MissionRun::start(plan, view, now)) {
            Ok((run, tick)) => {
                self.missions.placing = None;
                self.note = format!("Mission started on rover {id} · {}", run.status());
                self.feed.record(serde_json::json!({
                    "kind": "mission_plan",
                    "rover_id": id,
                    "plan": run.plan,
                }));
                self.missions.runs.insert(id, run);
                self.apply_mission(id, tick);
            }
            Err(reason) => self.note = reason,
        }
    }

    fn mission_action(
        &mut self,
        id: u64,
        act: impl FnOnce(&mut MissionRun, &MissionView, f64) -> Result<MissionTick, String>,
    ) {
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id).cloned() else {
            return;
        };
        let now = self.now;
        let Some(run) = self.missions.runs.get_mut(&id) else {
            return;
        };
        match with_view(&mark, |view| act(run, view, now)) {
            Ok(tick) => {
                self.note = format!("Rover {id}: {}", run.status());
                self.apply_mission(id, tick);
            }
            Err(reason) => self.note = reason,
        }
    }

    /// Stops the rover's mission, if one runs (Stop, takeover, fleet switch).
    pub(super) fn halt_mission(&mut self, id: u64, why: &str) {
        if let Some(run) = self.missions.runs.get_mut(&id).filter(|run| run.active()) {
            let tick = run.pause_now(why);
            self.apply_mission(id, tick);
        }
    }
}

/// Mission tab body for the selected rover.
pub(super) fn mission_tab(
    dashboard: &Dashboard,
    mark: &RoverMark,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
    let board = &dashboard.missions;
    let run = board.runs.get(&id).filter(|run| run.active());
    let plan = board.plans.get(&id).cloned().unwrap_or_default();
    let block = with_view(mark, mission_block);
    let last = board
        .runs
        .get(&id)
        .filter(|run| !run.active())
        .map(MissionRun::status);
    v_flex()
        .gap_5()
        .child(run_section(id, run, block.as_deref(), last, &plan, cx))
        .child(plan_section(
            dashboard,
            mark,
            &plan,
            run.map(|run| run.index),
            cx,
        ))
}

fn run_section(
    id: u64,
    run: Option<&MissionRun>,
    block: Option<&str>,
    last: Option<String>,
    plan: &MissionPlan,
    cx: &mut Context<Dashboard>,
) -> Div {
    let Some(run) = run else {
        return section("Mission", cx)
            .child(div().text_sm().child(if plan.steps.is_empty() {
                "No plan yet".to_string()
            } else {
                format!(
                    "Ready · {} steps{}",
                    plan.steps.len(),
                    if plan.repeat { ", repeating" } else { "" }
                )
            }))
            .child(
                h_flex().gap_2().child(
                    Button::new(("mission-start", id))
                        .primary()
                        .small()
                        .label("Start mission")
                        .disabled(plan.steps.is_empty())
                        .tooltip("Sends step 1 now. Needs L2 Direct or L3 Planned.")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.start_mission(id);
                            cx.notify();
                        })),
                ),
            )
            .when_some(block.filter(|_| !plan.steps.is_empty()), |section, why| {
                section.child(caution(why.to_string(), cx))
            })
            .when_some(last, |section, last| {
                section.child(hint(format!("Last run: {last}"), cx))
            });
    };
    let paused = matches!(run.phase, Phase::Paused(_));
    let waiting = run.phase == Phase::Waiting(Wait::Refill);
    section("Mission", cx)
        .child(div().text_sm().font_medium().child(run.status()))
        .when(waiting, |section| {
            section.child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new(("mission-loaded", id))
                            .primary()
                            .small()
                            .label("Loaded · continue")
                            .tooltip("The human loaded the dumper. Go on to the next step.")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.mission_action(id, |run, view, now| Ok(run.loaded(view, now)));
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("mission-empty", id))
                            .outline()
                            .small()
                            .label("Out of balls")
                            .tooltip("No balls left at home. Go back to the first Collect step.")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.mission_action(id, |run, view, now| {
                                    Ok(run.out_of_balls(view, now))
                                });
                                cx.notify();
                            })),
                    ),
            )
        })
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(if paused {
                    Button::new(("mission-resume", id))
                        .primary()
                        .small()
                        .label("Resume")
                        .tooltip("Runs the current step again from its start")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mission_action(id, |run, view, now| run.resume(view, now));
                            cx.notify();
                        }))
                } else {
                    Button::new(("mission-pause", id))
                        .outline()
                        .small()
                        .label("Pause")
                        .tooltip("Cancels the goal or dock in flight; Resume repeats this step")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mission_action(id, |run, _, _| {
                                Ok(run.pause_now("operator paused"))
                            });
                            cx.notify();
                        }))
                })
                .child(
                    Button::new(("mission-skip", id))
                        .outline()
                        .small()
                        .label("Skip step")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mission_action(id, |run, view, now| Ok(run.skip(view, now)));
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(("mission-stop", id))
                        .outline()
                        .small()
                        .label("End mission")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mission_action(id, |run, _, _| Ok(run.stop()));
                            cx.notify();
                        })),
                ),
        )
        .child(hint(
            "Dumps still need your approval at L0–L3. Stop pauses the mission.",
            cx,
        ))
}

fn plan_section(
    dashboard: &Dashboard,
    mark: &RoverMark,
    plan: &MissionPlan,
    current: Option<usize>,
    cx: &mut Context<Dashboard>,
) -> Div {
    let id = mark.id;
    let placing = dashboard.missions.placing;
    let placeable = StepKind::PLACEABLE;
    let found = mark
        .objects
        .iter()
        .filter(|object| object.object.class == "bucket")
        .map(|object| {
            (
                object.object.id,
                bucket_label(&object.object),
                object.object.x,
                object.object.y,
            )
        })
        .collect::<Vec<_>>();
    let running = current.is_some();
    section("Plan", cx)
        .child(
            ButtonGroup::new(("mission-place", id))
                .outline()
                .small()
                .children(placeable.iter().enumerate().map(|(index, kind)| {
                    Button::new(("mission-kind", index as u64))
                        .label(kind.label())
                        .picked(placing == Some(*kind))
                }))
                .on_click(cx.listener(move |this, clicks: &Vec<usize>, _, cx| {
                    if this.selected == Some(id)
                        && let Some(kind) = clicks.first().and_then(|i| StepKind::PLACEABLE.get(*i))
                    {
                        this.missions.placing = if this.missions.placing == Some(*kind) {
                            None
                        } else {
                            Some(*kind)
                        };
                        this.drag = None;
                        this.note = match this.missions.placing {
                            Some(kind) => format!(
                                "Click the map to add {} steps · click {} again to stop",
                                kind.label().to_lowercase(),
                                kind.label()
                            ),
                            None => "Stopped placing steps".into(),
                        };
                    }
                    cx.notify();
                })),
        )
        .child(hint(
            match placing {
                Some(kind) => format!(
                    "Placing {} steps: each map click adds one. Click {} again to stop.",
                    kind.label().to_lowercase(),
                    kind.label()
                ),
                None => "Pick a kind, then click the map to add steps in order. Home waits there for the human refill.".into(),
            },
            cx,
        ))
        .when(!found.is_empty(), |section| {
            section.child(
                h_flex().gap_2().flex_wrap().children(found.into_iter().map(
                    |(bucket, label, x, y)| {
                        let text = format!("+ Score at {label}");
                        Button::new(("mission-score", u64::from(bucket)))
                            .outline()
                            .xsmall()
                            .label(text)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let plan = this.missions.plans.entry(id).or_default();
                                this.note = match plan
                                    .push(MissionStep::score(bucket, label.clone(), x, y))
                                {
                                    Ok(()) => format!("Step {} added: score at {label}", plan.steps.len()),
                                    Err(reason) => reason.into(),
                                };
                                cx.notify();
                            }))
                    },
                )),
            )
        })
        .children(plan.steps.iter().enumerate().map(|(index, step)| {
            step_row(id, index, step, current == Some(index), plan.steps.len(), cx)
        }))
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(("mission-repeat", id))
                        .outline()
                        .small()
                        .label("Repeat")
                        .picked(plan.repeat)
                        .tooltip("After the last step, start again from step 1")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let plan = this.missions.plans.entry(id).or_default();
                            plan.repeat = !plan.repeat;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(("mission-clear", id))
                        .outline()
                        .small()
                        .label("Clear plan")
                        .disabled(plan.steps.is_empty())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.missions.plans.remove(&id);
                            this.note = "Plan cleared".into();
                            cx.notify();
                        })),
                ),
        )
        .when(running, |section| {
            section.child(hint(
                "Edits apply the next time you start the mission.",
                cx,
            ))
        })
}

fn step_row(
    id: u64,
    index: usize,
    step: &MissionStep,
    current: bool,
    count: usize,
    cx: &Context<Dashboard>,
) -> impl IntoElement {
    let key = id.saturating_mul(1000).saturating_add(index as u64);
    h_flex()
        .gap_2()
        .justify_between()
        .child(
            h_flex()
                .gap_2()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .size(px(10.))
                        .rounded_sm()
                        .flex_none()
                        .bg(rgb(step.kind.color())),
                )
                .child(
                    div()
                        .text_sm()
                        .min_w_0()
                        .when(current, |text| {
                            text.font_semibold().text_color(cx.theme().success)
                        })
                        .child(format!("{}. {}", index + 1, step.describe())),
                ),
        )
        .child(
            h_flex()
                .gap_1()
                .flex_none()
                .child(
                    Button::new(("mission-up", key))
                        .ghost()
                        .xsmall()
                        .label("↑")
                        .disabled(index == 0)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(plan) = this.missions.plans.get_mut(&id) {
                                plan.move_up(index);
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(("mission-down", key))
                        .ghost()
                        .xsmall()
                        .label("↓")
                        .disabled(index + 1 >= count)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(plan) = this.missions.plans.get_mut(&id) {
                                plan.move_down(index);
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(("mission-remove", key))
                        .ghost()
                        .xsmall()
                        .label("✕")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(plan) = this.missions.plans.get_mut(&id) {
                                plan.remove(index);
                            }
                            cx.notify();
                        })),
                ),
        )
}
