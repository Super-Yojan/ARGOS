//! The six levels of autonomy and the rover's available controllers.
//!
//! Top panel: the six levels as a row of buttons. The filled button is the
//! level the rover is at right now. The outlined blue one is the level its
//! next goal will be driven at, when that differs. Levels the rover's
//! software cannot run are greyed out.
//!
//! Bottom panel: every controller reported by the rover's `Capabilities`,
//! and a selector to choose which controller each autonomy level (L1–L3)
//! uses.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use terra_comms::{GoalKind, StageOption};

use super::widgets::{ButtonLook, caps_label, heading, mono, note, panel};
use super::{ActionHandler, action_button};
use crate::fleet::{Level, Request, Rover, RoverAction};
use crate::theme;

#[derive(IntoElement)]
pub struct LevelPicker {
    rover: Rover,
    on_action: ActionHandler,
}

impl LevelPicker {
    pub fn new(rover: Rover, on_action: ActionHandler) -> Self {
        Self { rover, on_action }
    }
}

impl RenderOnce for LevelPicker {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self { rover, on_action } = self;

        let buttons = Level::ALL.into_iter().map(|level| {
            let look = if level == rover.level {
                ButtonLook::Primary
            } else {
                ButtonLook::Outline
            };
            let button = action_button(
                &rover,
                RoverAction::SetLevel(level),
                level.code(),
                look,
                &on_action,
            )
            .flex_1()
            .px_0();
            let is_goal_level = level == rover.goal_level && level != rover.level;
            if is_goal_level {
                button
                    .border_color(rgb(theme::BLUE))
                    .text_color(rgb(theme::BLUE_TEXT))
            } else {
                button
            }
        });

        let summary = format!(
            "now {} ({}) · goals at {}",
            rover.level.code(),
            rover.active_controller,
            rover.goal_level.code()
        );
        let unavailable: Vec<&str> = Level::ALL
            .into_iter()
            .filter(|level| !rover.available_levels.contains(level))
            .map(Level::code)
            .collect();

        let level_panel = panel()
            .child(
                h_flex()
                    .justify_between()
                    .items_baseline()
                    .gap_2()
                    .child(heading("Autonomy level"))
                    .child(mono(summary).text_color(rgb(theme::TEXT_MUTED))),
            )
            .child(h_flex().gap_1().children(buttons))
            .child(note(format!(
                "{}: {}",
                rover.goal_level.label(),
                rover.goal_level.description()
            )))
            .children((!unavailable.is_empty()).then(|| {
                note(format!(
                    "{} not offered: this rover's software cannot run it.",
                    unavailable.join(", ")
                ))
            }));

        v_flex()
            .gap_3()
            .child(level_panel)
            .child(controllers_panel(&rover, &on_action))
    }
}

fn controllers_panel(rover: &Rover, on_action: &ActionHandler) -> Div {
    let count_label = if rover.controllers.is_empty() {
        "waiting…".to_string()
    } else {
        format!("{} available", rover.controllers.len())
    };

    let header = h_flex()
        .justify_between()
        .items_baseline()
        .child(heading("Controllers"))
        .child(mono(count_label).text_color(rgb(theme::TEXT_MUTED)));

    if rover.controllers.is_empty() {
        return panel().child(header).child(note(
            "Waiting for this rover to report its available controllers.",
        ));
    }

    let available_list = v_flex().gap(px(6.0)).children(
        rover
            .controllers
            .iter()
            .map(|stage| controller_row(stage, stage.id == rover.active_controller)),
    );

    let assignments = v_flex().gap_2().children(
        Level::CONFIGURABLE
            .into_iter()
            .map(|level| level_controller_row(rover, level, on_action)),
    );

    panel()
        .gap(px(10.0))
        .child(header)
        .child(v_flex().gap(px(6.0)).child(caps_label("Available")).child(available_list))
        .child(
            div()
                .h(px(1.0))
                .w_full()
                .bg(rgb(theme::BORDER)),
        )
        .child(
            v_flex()
                .gap(px(6.0))
                .child(caps_label("Controller per level"))
                .child(assignments)
                .child(note("L0 Manual uses direct stick (teleop).")),
        )
}

fn controller_row(stage: &StageOption, active: bool) -> Div {
    let goals = if stage.goals.is_empty() {
        "stick".to_string()
    } else {
        stage
            .goals
            .iter()
            .map(|kind| match kind {
                GoalKind::Local => "local",
                GoalKind::Gps => "gps",
                GoalKind::Explore => "explore",
            })
            .collect::<Vec<_>>()
            .join("+")
    };

    let mut top = h_flex()
        .justify_between()
        .items_center()
        .gap_2()
        .child(
            h_flex()
                .items_baseline()
                .gap(px(6.0))
                .child(
                    mono(stage.id.clone())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::TEXT)),
                )
                .child(
                    mono(format!("v{}", stage.version))
                        .text_size(px(11.0))
                        .text_color(rgb(theme::TEXT_MUTED)),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap(px(6.0))
                .when(active, |row| {
                    row.child(
                        mono("active")
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::GREEN)),
                    )
                })
                .child(
                    mono(goals)
                        .text_size(px(11.0))
                        .text_color(rgb(theme::TEXT_MUTED)),
                ),
        );

    if active {
        top = top.text_color(rgb(theme::TEXT));
    }

    v_flex()
        .px_2()
        .py(px(6.0))
        .gap(px(2.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(if active { theme::GREEN } else { theme::BORDER }))
        .child(top)
        .when(!stage.description.is_empty(), |card| {
            card.child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(stage.description.clone()),
            )
        })
}

fn level_controller_row(rover: &Rover, level: Level, on_action: &ActionHandler) -> Div {
    let assigned = rover.controller_for(level);
    let planner_hint = match level.default_planner() {
        Some(planner) => format!("planner: {planner}"),
        None => "no planner".to_string(),
    };

    let options: Vec<&StageOption> = rover
        .controllers
        .iter()
        .filter(|stage| {
            rover.allows(&RoverAction::SetLevelController {
                level,
                controller: stage.id.clone(),
            })
        })
        .collect();

    let assigned_missing = assigned
        .is_some_and(|name| !options.iter().any(|stage| stage.id == name));

    let pills = options.into_iter().map(|stage| {
        let selected = assigned == Some(stage.id.as_str());
        let id = SharedString::from(format!("ctrl-{}-{}-{}", rover.id, level.code(), stage.id));
        let request = Request {
            rover: rover.id,
            action: RoverAction::SetLevelController {
                level,
                controller: stage.id.clone(),
            },
        };
        let on_action = on_action.clone();

        let pill = div()
            .id(id)
            .px_2()
            .py(px(4.0))
            .rounded(px(6.0))
            .border_1()
            .font_family(theme::FONT_MONO)
            .text_size(px(12.0))
            .cursor_pointer()
            .hover(|style| style.opacity(0.85))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_action(&request, window, cx);
            })
            .child(stage.id.clone());

        if selected {
            pill.bg(rgb(theme::BLUE_BG))
                .border_color(rgb(theme::BLUE))
                .text_color(rgb(theme::BLUE_TEXT))
                .font_weight(FontWeight::SEMIBOLD)
        } else {
            pill.border_color(rgb(theme::BORDER))
                .text_color(rgb(theme::TEXT_MUTED))
        }
    });

    v_flex()
        .gap(px(4.0))
        .child(
            h_flex()
                .justify_between()
                .items_baseline()
                .child(
                    mono(level.label())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::TEXT)),
                )
                .child(
                    mono(planner_hint)
                        .text_size(px(11.0))
                        .text_color(rgb(theme::TEXT_MUTED)),
                ),
        )
        .child(h_flex().flex_wrap().gap_1().children(pills))
        .when(assigned_missing, |col| {
            col.child(
                mono(format!(
                    "\"{}\" is not offered by this rover — pick one above",
                    assigned.unwrap_or_default()
                ))
                .text_size(px(11.0))
                .text_color(rgb(theme::ORANGE_TEXT)),
            )
        })
}
