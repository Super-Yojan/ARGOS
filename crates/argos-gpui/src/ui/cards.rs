//! Rover callout cards on the map and tabs for rovers off screen or with
//! no position.

use super::widgets::Picked;
use crate::Dashboard;

use argos_gpui::console::{self, CalloutTag};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{ActiveTheme, Sizable, StyledExt, button::Button, tag::Tag};
use gpui_kit::{prelude::*, *};

pub(super) fn callout_tag(tag: CalloutTag) -> Tag {
    match tag {
        CalloutTag::Stopped => Tag::danger(),
        CalloutTag::BackingUp => Tag::warning(),
        CalloutTag::Driving => Tag::info(),
        CalloutTag::Attention => Tag::warning(),
        CalloutTag::Goal => Tag::secondary(),
    }
    .small()
    .child(tag.label())
}

/// Widest a rover card draws, so placement can keep it clear of the
/// inspector. Longer states truncate.
pub(super) const CARD_WIDTH_REM: f32 = 14.;

/// One card's height plus the gap between stacked cards.
pub(super) const CARD_HEIGHT_REM: f32 = 3.5;

/// Rover cards that share one projected anchor, stacked beside it on the side
/// `place_cards` chose to stay inside the field and clear of the inspector. Choosing a card selects
/// that rover, as clicking the rover does.
pub(super) fn card_group(
    group: console::CardGroup,
    field: &[(u64, Option<Point<Pixels>>, console::Callout)],
    selected: Option<u64>,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let cards: Vec<_> = group
        .ids
        .iter()
        .filter_map(|id| field.iter().find(|(row, _, _)| row == id))
        .map(|(id, _, callout)| {
            rover_card(*id, callout.clone(), selected == Some(*id), cx).into_any_element()
        })
        .collect();
    div()
        .absolute()
        .left(px(group.x))
        .top(px(group.y))
        .size_0()
        .child(
            v_flex()
                .absolute()
                .gap_1()
                .map(|stack| {
                    if group.below {
                        stack.top_4()
                    } else {
                        stack.bottom_4()
                    }
                })
                .map(|stack| {
                    if group.leading {
                        stack.right_4().items_end()
                    } else {
                        stack.left_4().items_start()
                    }
                })
                // Keep the map from starting an orbit under the cards.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .children(cards),
        )
}

fn rover_card(
    id: u64,
    callout: console::Callout,
    selected: bool,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    Button::new(("callout", id))
        .outline()
        .small()
        .h_auto()
        .py_1()
        .max_w(rems(CARD_WIDTH_REM))
        .bg(cx.theme().popover)
        .picked(selected)
        .child(
            v_flex()
                .items_start()
                .gap_1()
                .min_w_0()
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().font_semibold().child(format!("Rover {id}")))
                        .children(callout.tag.map(callout_tag)),
                )
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_color(cx.theme().muted_foreground)
                        .child(callout.state),
                ),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            this.focus.focus(window, cx);
            this.select(id);
            cx.notify();
        }))
}

/// Rovers whose position is outside the current view, then fleet members
/// with no position at all. Choosing an off-screen rover centers the map on
/// it and selects it; an unlocated one can only say why it has no position.
/// Kept on the leading edge so the inspector never covers it.
pub(super) fn offscreen_tabs(
    field: &[(u64, Option<Point<Pixels>>, console::Callout)],
    unlocated: &[console::Unlocated],
    selected: Option<u64>,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    v_flex()
        .absolute()
        .left_3()
        .bottom_3()
        .items_start()
        .gap_2()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .children(
            field
                .iter()
                .filter(|(_, at, _)| at.is_none())
                .map(|(id, _, callout)| {
                    let id = *id;
                    Button::new(("offscreen", id))
                        .outline()
                        .small()
                        .bg(cx.theme().popover)
                        .picked(selected == Some(id))
                        .tooltip("Off screen. Centers the map on this rover and selects it.")
                        .child(
                            h_flex()
                                .gap_2()
                                .child(format!("Rover {id}"))
                                .children(callout.tag.map(callout_tag))
                                .child(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(callout.state.clone()),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.focus.focus(window, cx);
                            if let Some(mark) = this.marks.iter().find(|mark| mark.id == id)
                                && let Err(error) =
                                    this.map.show_lat_lon(mark.latitude, mark.longitude)
                            {
                                this.note = error.to_string();
                            }
                            this.select(id);
                            cx.notify();
                        }))
                }),
        )
        .children(unlocated.iter().map(|row| {
            let id = row.id;
            let detail: SharedString = row.detail.clone().into();
            Button::new(("unlocated", id))
                .outline()
                .small()
                .bg(cx.theme().popover)
                .tooltip(detail.clone())
                .child(
                    h_flex()
                        .gap_2()
                        .child(row.label.clone())
                        .child(Tag::secondary().small().child("No position")),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.focus.focus(window, cx);
                    this.note = format!("Rover {id} · {detail}");
                    cx.notify();
                }))
        }))
}
