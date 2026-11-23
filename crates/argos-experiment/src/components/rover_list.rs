//! A compact list of rovers that need nothing: one row each, with the
//! level of autonomy and what the rover is doing. Click a row to look at
//! that rover.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::ActionHandler;
use super::widgets::mono;
use crate::fleet::{Activity, Request, Rover, RoverAction};
use crate::theme;

#[derive(IntoElement)]
pub struct RoverList {
    rovers: Vec<Rover>,
    /// Drawn with a blue frame.
    selected: Option<u64>,
    on_action: ActionHandler,
}

impl RoverList {
    pub fn new(rovers: Vec<Rover>, on_action: ActionHandler) -> Self {
        Self {
            rovers,
            selected: None,
            on_action,
        }
    }

    pub fn selected(mut self, rover_id: Option<u64>) -> Self {
        self.selected = rover_id;
        self
    }
}

impl RenderOnce for RoverList {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            rovers,
            selected,
            on_action,
        } = self;
        v_flex()
            .id("rover-list")
            .gap(px(6.0))
            .children(rovers.iter().map(|rover| {
                let on_action = on_action.clone();
                let focus = Request {
                    rover: rover.id,
                    action: RoverAction::Focus,
                };
                row(rover)
                    .when(selected == Some(rover.id), |row| {
                        row.border_color(rgb(theme::BLUE))
                    })
                    .on_click(move |_, window, cx| on_action(&focus, window, cx))
            }))
    }
}

fn row(rover: &Rover) -> Stateful<Div> {
    let detail = match rover.activity {
        Activity::Driving { distance } => format!("{distance:.1} m"),
        _ => String::new(),
    };
    let running = matches!(rover.activity, Activity::Driving { .. });

    h_flex()
        .id(("rover-row", rover.id as usize))
        .min_h(px(44.0))
        .px(px(10.0))
        .gap_2()
        .items_center()
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .bg(rgb(theme::PANEL))
        .cursor_pointer()
        .child(
            div()
                .w(px(76.0))
                .flex_none()
                .font_weight(FontWeight::BOLD)
                .child(rover.name()),
        )
        .child(
            mono(rover.level.code())
                .w(px(36.0))
                .flex_none()
                .text_color(rgb(theme::BLUE_TEXT)),
        )
        .child(
            div()
                .flex_1()
                .text_size(px(13.0))
                .text_color(rgb(if running {
                    theme::GREEN
                } else {
                    theme::TEXT_MUTED
                }))
                .child(rover.headline()),
        )
        .child(mono(detail).text_color(rgb(theme::TEXT_MUTED)))
}
