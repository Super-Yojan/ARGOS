//! L4 Explore and L5 Task sections of the Overview tab.

use super::levels::gps_blocks;
use super::widgets::Picked;
use super::widgets::caution;
use super::widgets::hint;
use super::widgets::rows;
use super::widgets::section;
use crate::Dashboard;

use argos_core::{EXPLORE_BUDGETS_MIN, TASK_BUDGETS_MIN, explore_refusal, task_refusal};
use argos_gpui::RoverMark;
use gpui_kit::base::h_flex;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonGroup},
    input::Input,
};
use gpui_kit::{prelude::*, *};

/// L4 budget picker and Terra's run status. Terra chooses the frontiers; the
/// target also shows as the rover's goal on the map.
pub(super) fn explore_section(
    dashboard: &Dashboard,
    mark: &RoverMark,
    stopped: bool,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let refusal = explore_refusal(stopped, gps_blocks(mark, "explore"));
    let exploring = mark.requested_level.as_deref() == Some("explore");
    section("Exploration · L4", cx)
        .child(
            ButtonGroup::new("explore-budget")
                .outline()
                .small()
                .children(EXPLORE_BUDGETS_MIN.iter().enumerate().map(|(index, minutes)| {
                    Button::new(("explore-budget", index as u64))
                        .label(format!("{minutes} min"))
                        .picked(dashboard.explore_budget == index)
                }))
                .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                    if let Some(index) = clicks
                        .first()
                        .filter(|index| **index < EXPLORE_BUDGETS_MIN.len())
                    {
                        this.explore_budget = *index;
                    }
                    cx.notify();
                })),
        )
        .when_some(refusal.filter(|_| !exploring), |section, reason| {
            section.child(caution(reason, cx))
        })
        .map(|section| match mark.exploration.as_ref() {
            Some(report) => {
                let stuck = report.running()
                    && matches!(
                        report.reason.as_deref(),
                        Some("frontier_retry" | "obstacle_blocked")
                    );
                section
                    .when(stuck, |section| {
                        section.child(caution(report.state_label(), cx))
                    })
                    .child(rows(report.rows()))
            }
            None => section.child(hint(
                "L4 Explore drives to unmapped frontiers on its own until the budget ends or no frontier is left, and maps the buckets and balls it sees under Found objects. Stop, Cancel goal, or choosing another level ends the run.",
                cx,
            )),
        })
}

/// The picked answerer if the rover has it, else its MCP agent, else the built-in model.
pub(super) fn task_agent_choice(
    picked: Option<argos_core::TaskAgent>,
    mark: &RoverMark,
) -> argos_core::TaskAgent {
    use argos_core::TaskAgent;
    picked
        .filter(|agent| mark.task_agents.contains(agent))
        .or_else(|| {
            mark.task_agents
                .contains(&TaskAgent::Mcp)
                .then_some(TaskAgent::Mcp)
        })
        .unwrap_or(TaskAgent::Builtin)
}

/// L5: type a task; the phone's model or MCP agent picks waypoints and Terra drives them.
/// The plan also shows as the rover's goal and route on the map.
pub(super) fn task_section(
    dashboard: &Dashboard,
    mark: &RoverMark,
    stopped: bool,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
    let refusal = task_refusal(stopped, gps_blocks(mark, "task"), mark.task_supported);
    let report = mark.task.clone();
    let running = report.as_ref().is_some_and(|task| task.running())
        && mark.requested_level.as_deref() == Some("task");
    let agents = mark.task_agents.clone();
    let chosen = task_agent_choice(dashboard.task_agent, mark);
    section("Task · L5", cx)
        .when(agents.len() > 1, |section| {
            section.child(
                ButtonGroup::new("task-agent")
                    .outline()
                    .small()
                    .children(agents.iter().enumerate().map(|(index, agent)| {
                        Button::new(("task-agent", index as u64))
                            .label(agent.label())
                            .picked(*agent == chosen)
                    }))
                    .on_click(cx.listener(move |this, clicks: &Vec<usize>, _, cx| {
                        if let Some(agent) = clicks.first().and_then(|index| agents.get(*index)) {
                            this.task_agent = Some(*agent);
                        }
                        cx.notify();
                    })),
            )
        })
        .child(Input::new(&dashboard.task_input).small())
        .child(
            h_flex()
                .gap_2()
                .child(
                    ButtonGroup::new("task-budget")
                        .outline()
                        .small()
                        .children(TASK_BUDGETS_MIN.iter().enumerate().map(|(index, minutes)| {
                            Button::new(("task-budget", index as u64))
                                .label(format!("{minutes} min"))
                                .picked(dashboard.task_budget == index)
                        }))
                        .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                            if let Some(index) = clicks
                                .first()
                                .filter(|index| **index < TASK_BUDGETS_MIN.len())
                            {
                                this.task_budget = *index;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(("task-start", id))
                        .small()
                        .label(if running { "Replace task" } else { "Start task" })
                        .disabled(refusal.is_some())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start_task(id, cx);
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .when(running, |row| {
                    let task_id = report
                        .as_ref()
                        .map(|task| task.task_id.clone())
                        .unwrap_or_default();
                    row.child(
                        Button::new(("task-cancel", id))
                            .outline()
                            .small()
                            .label("Cancel task")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.feed.cancel_task(id, task_id.clone());
                                this.note = "Task cancel requested".into();
                                cx.notify();
                            })),
                    )
                }),
        )
        .when_some(refusal, |section, reason| section.child(caution(reason, cx)))
        .map(|section| match report {
            Some(task) => {
                let waiting = running && task.waiting_for_operator();
                let drive_asked = running && task.awaiting_drive_approval();
                let driving = running && task.drive_remaining_s.is_some();
                let asker = match task.agent {
                    argos_core::TaskAgent::Builtin => "Model",
                    argos_core::TaskAgent::Mcp => "Agent",
                };
                let drive_task_id = task.task_id.clone();
                let question = task.question.clone();
                let recent: Vec<String> = task
                    .history
                    .iter()
                    .rev()
                    .take(4)
                    .rev()
                    .map(|(kind, text)| format!("{kind}: {text}"))
                    .collect();
                section
                    .when_some(question.filter(|_| waiting), |section, question| {
                        section.child(caution(format!("{asker} asks: {question}"), cx))
                    })
                    .when_some(
                        task.drive_request.clone().filter(|_| drive_asked),
                        |section, request| {
                            section.child(caution(
                                format!(
                                    "Agent asks for raw drive for {:.0} s: {}. Speed is capped at 0.5 m/s and the obstacle planner still checks every command.",
                                    request.seconds, request.reason
                                ),
                                cx,
                            ))
                        },
                    )
                    .when(drive_asked || driving, |section| {
                        let deny_id = drive_task_id.clone();
                        section.child(
                            h_flex()
                                .gap_2()
                                .when(drive_asked, |row| {
                                    row.child(
                                        Button::new(("task-drive-approve", id))
                                            .small()
                                            .label("Approve drive")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.feed.publish_task_drive(
                                                    id,
                                                    drive_task_id.clone(),
                                                    true,
                                                );
                                                this.note = "Approving raw drive".into();
                                                cx.notify();
                                            })),
                                    )
                                })
                                .child(
                                    Button::new(("task-drive-deny", id))
                                        .outline()
                                        .small()
                                        .label(if driving { "End drive" } else { "Deny drive" })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.feed.publish_task_drive(id, deny_id.clone(), false);
                                            this.note = "Denying raw drive".into();
                                            cx.notify();
                                        })),
                                ),
                        )
                    })
                    .when(running, |section| {
                        section.child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().child(Input::new(&dashboard.task_reply).small()))
                                .child(
                                    Button::new(("task-reply", id))
                                        .outline()
                                        .small()
                                        .label(if waiting { "Answer" } else { "Guide" })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.reply_task(id, window, cx);
                                            cx.notify();
                                        })),
                                ),
                        )
                    })
                    .child(rows(task.rows()))
                    .children(recent.into_iter().map(|line| hint(line, cx)))
            }
            None => section.child(hint(
                "L5 hands the task to the rover's on-phone language model or MCP agent (Qwen 3.5 2B). It reads the phone's map, picks waypoints, and asks you when unsure. Terra checks every waypoint and drives it with the L3 planner. The agent can ask for raw drive, which runs only after you approve it. Stop, Cancel task, or another level ends it.",
                cx,
            )),
        })
}
