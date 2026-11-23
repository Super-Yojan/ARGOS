//! "Needs you": one card per rover that waits for the operator, most
//! urgent first. The number on a card matches the number on the map.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::widgets::{ButtonLook, clock, mono};
use super::{ActionHandler, action_button};
use crate::fleet::{Request, Rover, RoverAction, Urgency};
use crate::theme::{self, Tone};

#[derive(IntoElement)]
pub struct HelpQueue {
    /// From [`crate::fleet::help_queue`]: already in order.
    queue: Vec<Rover>,
    commands_only: bool,
    on_action: ActionHandler,
}

impl HelpQueue {
    pub fn new(queue: Vec<Rover>, on_action: ActionHandler) -> Self {
        Self {
            queue,
            commands_only: false,
            on_action,
        }
    }

    /// Offer only the buttons that send a command (resume, clear, release),
    /// not the ones that need the big map or the drive view. For places
    /// where the operator is busy with something else.
    pub fn commands_only(mut self) -> Self {
        self.commands_only = true;
        self
    }
}

impl RenderOnce for HelpQueue {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            queue,
            commands_only,
            on_action,
        } = self;
        v_flex().id("help-queue").gap_2().children(
            queue
                .iter()
                .enumerate()
                .map(|(index, rover)| card(index + 1, rover, commands_only, &on_action)),
        )
    }
}

/// The colours for a rover that needs help, by how urgent it is.
pub fn tone(urgency: Urgency) -> Tone {
    match urgency {
        Urgency::Stopped => theme::RED,
        Urgency::Waiting => theme::ORANGE,
    }
}

fn card(
    place: usize,
    rover: &Rover,
    commands_only: bool,
    on_action: &ActionHandler,
) -> Stateful<Div> {
    let stopped = rover.urgency() == Some(Urgency::Stopped);
    let tone = rover.urgency().map(tone).unwrap_or(theme::ORANGE);

    let title = h_flex()
        .justify_between()
        .items_start()
        .gap_2()
        .child(
            // In a narrow column the name wraps; the clock keeps its place.
            h_flex()
                .flex_1()
                .min_w_0()
                .flex_wrap()
                .gap_x_2()
                .font_family(theme::FONT_HEADING)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(16.0))
                .child(
                    mono(place.to_string())
                        .text_size(px(16.0))
                        .text_color(rgb(tone.fill)),
                )
                .child(format!("{} · {}", rover.name(), rover.headline())),
        )
        .child(
            mono(clock(rover.waiting))
                .flex_none()
                .text_size(px(16.0))
                .text_color(rgb(tone.text)),
        );

    let (main, second) = rover
        .suggested_actions()
        .unwrap_or((RoverAction::TakeOver, RoverAction::Focus));
    let offered = |action: &RoverAction| !commands_only || is_command(action);
    let mut buttons = h_flex().gap_2();
    if offered(&main) {
        let label = main.label();
        buttons = buttons.child(
            action_button(rover, main, label, ButtonLook::ToneFilled(tone), on_action).flex_1(),
        );
    }
    if offered(&second) {
        let label = second.label();
        buttons = buttons.child(
            action_button(rover, second, label, ButtonLook::ToneOutline(tone), on_action).flex_1(),
        );
    }

    let on_action = on_action.clone();
    let focus = Request {
        rover: rover.id,
        action: RoverAction::Focus,
    };
    v_flex()
        .id(("help-card", rover.id as usize))
        .gap_2()
        .p_3()
        .rounded(px(10.0))
        .bg(rgb(tone.bg))
        .border_color(rgb(tone.accent))
        // A stopped rover gets the heavier frame.
        .map(|card| {
            if stopped {
                card.border_2()
            } else {
                card.border_1()
            }
        })
        .cursor_pointer()
        .on_click(move |_, window, cx| on_action(&focus, window, cx))
        .child(title)
        .child(
            div()
                .text_size(px(13.0))
                .text_color(rgb(tone.text))
                .child(rover.explanation()),
        )
        .child(buttons)
}

fn is_command(action: &RoverAction) -> bool {
    !matches!(
        action,
        RoverAction::Focus | RoverAction::TakeOver | RoverAction::SetGoal
    )
}
