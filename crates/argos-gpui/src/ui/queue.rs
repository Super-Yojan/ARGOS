//! The right panel: what needs the operator now (ranked), what will soon,
//! and what was just resolved.

use super::palette;
use crate::Dashboard;

use argos_core::REJECT_REASONS;
use argos_gpui::attention::{self, Item, Need, Upcoming};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt, button::Button, scroll::ScrollableElement,
};
use gpui_kit::{prelude::*, *};

/// Panel width in pixels.
pub(super) const QUEUE_WIDTH_PX: f32 = 400.;

impl Dashboard {
    /// Everything waiting on the operator: rover stops, holds, task
    /// questions, and the agent's open recommendations.
    pub(super) fn attention_items(&self) -> Vec<Item> {
        let mut items = attention::from_marks(&self.marks, |mark| self.stop_cause(mark));
        for entry in self.agent.book.open() {
            let Some(mark) = self.marks.iter().find(|mark| mark.id == entry.rec.rover) else {
                continue;
            };
            items.push(Item {
                key: format!("rec:{}", entry.rec.rec_id),
                rover: entry.rec.rover,
                level: attention::level_chip(mark),
                need: Need::Advice,
                urgency: entry.rec.urgency,
                text: format!("{}. {}", entry.rec.action.title(), entry.rec.reason),
                rec_id: Some(entry.rec.rec_id.clone()),
            });
        }
        items
    }

    pub(super) fn track_attention(&mut self) {
        let items = self.attention_items();
        self.attention.update(&items, self.now);
    }
}

fn need_color(need: Need, urgency: u8) -> u32 {
    match (need, urgency) {
        (Need::Stopped, _) | (_, 3) => palette::DANGER,
        (_, 2) => palette::WARNING,
        _ => palette::MUTED,
    }
}

fn chip(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex_none()
        .px_1p5()
        .py_0p5()
        .rounded(px(3.))
        .border_1()
        .border_color(cx.theme().border)
        .text_xs()
        .font_family(cx.theme().mono_font_family.clone())
        .child(text.into())
}

fn heading(title: &'static str, note: &'static str, cx: &App) -> Div {
    h_flex()
        .justify_between()
        .items_baseline()
        .child(div().text_lg().font_semibold().child(title))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(note),
        )
}

pub(super) fn queue_panel(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let now = dashboard.now;
    let ranked = dashboard.attention.ranked(now);
    let upcoming = attention::upcoming(
        &dashboard.marks,
        dashboard.selected.filter(|_| dashboard.driving),
    );
    let resolved: Vec<_> = dashboard.attention.resolved().take(6).cloned().collect();
    let rejecting = dashboard.agent.rejecting().map(str::to_owned);
    let selected = dashboard.selected;
    let rows: Vec<AnyElement> = ranked
        .into_iter()
        .enumerate()
        .map(|(index, (item, waited))| {
            let open_reasons =
                item.rec_id.is_some() && item.rec_id.as_deref() == rejecting.as_deref();
            need_row(index + 1, item, waited, selected, open_reasons, cx).into_any_element()
        })
        .collect();
    let empty = rows.is_empty();
    v_flex()
        .flex_none()
        .w(px(QUEUE_WIDTH_PX))
        .h_full()
        .bg(cx.theme().popover)
        .border_l_1()
        .border_color(cx.theme().border)
        .child(
            v_flex()
                .id("queue-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(div().px_4().pt_4().pb_3().child(heading(
                    "Needs you",
                    "ranked by urgency, then wait",
                    cx,
                )))
                .when(empty, |panel| {
                    panel.child(
                        div()
                            .px_4()
                            .pb_4()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Nothing is waiting on you."),
                    )
                })
                .children(rows)
                .child(
                    v_flex()
                        .px_4()
                        .pt_4()
                        .pb_3()
                        .gap_2()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .child(heading("Coming up", "time until it needs you", cx))
                        .when(upcoming.is_empty(), |section| {
                            section.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Nothing scheduled."),
                            )
                        })
                        .children(upcoming.into_iter().map(|row| upcoming_row(row, cx))),
                )
                .child(
                    v_flex()
                        .px_4()
                        .pt_4()
                        .pb_3()
                        .gap_2()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .child(heading("Resolved", "last 10 min · wait", cx))
                        .when(resolved.is_empty(), |section| {
                            section.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Nothing resolved yet."),
                            )
                        })
                        .children(resolved.into_iter().map(|row| {
                            h_flex()
                                .gap_3()
                                .py_1()
                                .text_sm()
                                .child(
                                    div()
                                        .flex_none()
                                        .w(px(26.))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("R{}", row.rover)),
                                )
                                .child(div().flex_1().min_w_0().truncate().child(row.text))
                                .child(
                                    div()
                                        .flex_none()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(attention::clock(row.waited_s)),
                                )
                        })),
                ),
        )
        .child(
            div()
                .flex_none()
                .px_4()
                .py_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Waits count from when an item first appeared in this window"),
        )
}

fn need_row(
    rank: usize,
    item: Item,
    waited: f64,
    selected: Option<u64>,
    reasons: bool,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = item.rover;
    let color = need_color(item.need, item.urgency);
    let is_selected = selected == Some(id);
    let key = item.key.clone();
    let mut actions: Vec<AnyElement> = Vec::new();
    let button = |tag: &str| Button::new(SharedString::from(format!("q-{key}-{tag}")));
    match (item.need, item.rec_id.clone()) {
        (_, Some(rec_id)) if reasons => {
            for (index, (wire, label)) in REJECT_REASONS.iter().enumerate() {
                let rec_id = rec_id.clone();
                actions.push(
                    button(&format!("why{index}"))
                        .outline()
                        .small()
                        .label(*label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.reject_card(&rec_id, wire);
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
        }
        (_, Some(rec_id)) => {
            let accept = rec_id.clone();
            actions.push(
                button("accept")
                    .small()
                    .label("Accept")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.accept_card(&accept);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
            actions.push(
                button("reject")
                    .outline()
                    .small()
                    .label("Reject")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.agent.set_rejecting(Some(rec_id.clone()));
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        (Need::Stopped, None) => {
            actions.push(
                button("reset")
                    .small()
                    .label("Reset stop")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select(id);
                        this.reset_safety(id);
                        this.focus.focus(window, cx);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        (Need::Question, None) => {
            actions.push(
                button("answer")
                    .small()
                    .label("Answer")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select(id);
                        this.details_open = true;
                        this.focus.focus(window, cx);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        (Need::Hold | Need::Advice, None) => {}
    }
    if item.rec_id.is_none() {
        actions.push(
            button("takeover")
                .outline()
                .small()
                .label("Take over")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select(id);
                    this.begin_drive(window, cx);
                    cx.notify();
                }))
                .into_any_element(),
        );
        actions.push(
            button("select")
                .outline()
                .small()
                .label("Show on map")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select(id);
                    if let Some(mark) = this.marks.iter().find(|mark| mark.id == id) {
                        let _ = this.map.show_lat_lon(mark.latitude, mark.longitude);
                    }
                    this.focus.focus(window, cx);
                    cx.notify();
                }))
                .into_any_element(),
        );
    }
    h_flex()
        .items_start()
        .gap_3()
        .px_4()
        .py_3()
        .border_t_1()
        .border_color(cx.theme().border)
        .border_l_2()
        .border_color(if is_selected {
            rgb(palette::ACCENT).into()
        } else {
            cx.theme().border
        })
        .when(is_selected, |row| row.bg(rgb(palette::RAISED)))
        .child(
            div()
                .flex_none()
                .w(px(16.))
                .text_lg()
                .text_color(cx.theme().muted_foreground)
                .child(rank.to_string()),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().font_semibold().child(format!("R{id}")))
                        .child(chip(item.level.clone(), cx))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(color))
                                .child(item.need.label()),
                        )
                        .child(div().flex_1())
                        .child(
                            v_flex()
                                .items_end()
                                .child(
                                    div()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_color(rgb(color))
                                        .child(attention::clock(waited)),
                                )
                                .child(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("WAITING")
                                        .text_size(px(9.)),
                                ),
                        ),
                )
                .child(div().text_sm().child(item.text))
                .child(h_flex().gap_2().flex_wrap().children(actions)),
        )
}

fn upcoming_row(row: Upcoming, cx: &App) -> impl IntoElement {
    let fraction = match (row.remaining_s, row.budget_s) {
        (Some(left), Some(budget)) if budget > 0. => (1. - left / budget).clamp(0., 1.) as f32,
        _ => 0.,
    };
    h_flex()
        .gap_3()
        .py_1()
        .items_center()
        .child(
            div()
                .flex_none()
                .w(px(26.))
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(format!("R{}", row.rover)),
        )
        .child(chip(row.level, cx))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .when(row.remaining_s.is_some(), |column| {
                    column.child(
                        div()
                            .h(px(3.))
                            .w_full()
                            .rounded_full()
                            .bg(rgb(palette::BORDER))
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(fraction))
                                    .rounded_full()
                                    .bg(rgb(palette::ACCENT)),
                            ),
                    )
                })
                .child(
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(cx.theme().muted_foreground)
                        .child(row.text),
                ),
        )
        .child(
            div()
                .flex_none()
                .font_family(cx.theme().mono_font_family.clone())
                .text_sm()
                .child(row.remaining_s.map_or("—".into(), attention::clock)),
        )
}
