//! Timeline panel: one row per rover over the last 5 minutes and the next 10,
//! with a fixed now line. Solid blue is interaction, amber is waiting on you,
//! a thin line is neglect (running alone), and ◇ marks when a rover is
//! predicted to need you next. Rows are in queue order; click a row to select
//! that rover. The model is `argos_gpui::timeline`.

use crate::Dashboard;
use argos_gpui::RoverMark;
use argos_gpui::timeline::{
    Forecast, Observation, Phase, Row, Span, TIMELINE_AHEAD_S, TIMELINE_PAST_S, clock,
};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{ActiveTheme, StyledExt};
use gpui_kit::{prelude::*, *};

const LABEL_WIDTH: f32 = 200.;
const ROW_HEIGHT: f32 = 30.;
const BAR_HEIGHT: f32 = 14.;
/// Below this, a rover is treated as stopped for arrival estimates (m/s).
const MIN_SPEED: f64 = 0.05;
/// Interaction: the accent blue.
const TALK: u32 = super::palette::ACCENT;
/// Waiting: amber fill and edge.
const WAIT_FILL: u32 = 0xf6d38b;
const WAIT_EDGE: u32 = super::palette::WARNING;
/// Neglect line and grid.
const LINE: u32 = 0x8a9aa2;
const GRID: u32 = 0xd8dfdd;

impl Dashboard {
    /// Called from `sync` every 100 ms.
    pub(super) fn sync_timeline(&mut self, now: f64) {
        for mark in &self.marks {
            let manual = matches!(
                mark.requested_level.as_deref(),
                Some("teleop" | "assisted_teleop")
            );
            let seen = Observation {
                online: mark.status != "offline" && mark.reporting.label().is_none(),
                help: self.feed.open_help(mark.id),
                manual,
                attended: self.selected == Some(mark.id),
                x: mark.local_x,
                y: mark.local_y,
            };
            self.timeline.observe(now, mark.id, &seen);
        }
        let ids: Vec<u64> = self.marks.iter().map(|mark| mark.id).collect();
        self.timeline.retain(|id| ids.contains(&id));
    }

    pub(super) fn timeline_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let now = self.started.elapsed().as_secs_f64();
        let rows = self.timeline.rows(now, |id| {
            self.marks
                .iter()
                .find(|mark| mark.id == id)
                .map(|mark| forecasts(mark, self.timeline.speed(id)))
                .unwrap_or_default()
        });
        let selected = self.selected;
        let waiting = rows
            .iter()
            .filter(|row| row.phase == Some(Phase::Waiting))
            .count();
        v_flex()
            .size_full()
            .p_3()
            .gap_2()
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .gap_3()
                    .flex_wrap()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "past 5 min · next 10 min · queue order · {waiting} waiting on you"
                            )),
                    )
                    .child(div().flex_1())
                    .child(legend(cx)),
            )
            .child(if rows.is_empty() {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No rovers yet. Each rover gets a row once it reports.")
                    .into_any_element()
            } else {
                v_flex()
                    .id("timeline-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows.into_iter().map(|row| {
                        let id = row.id;
                        h_flex()
                            .id(("timeline-row", id))
                            .h(px(ROW_HEIGHT))
                            .w_full()
                            .cursor_pointer()
                            .rounded_sm()
                            .when(selected == Some(id), |row| row.bg(rgb(GRID)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select(id);
                                cx.notify();
                            }))
                            .child(row_label(&row, now, cx))
                            .child(track(&row, now))
                    }))
                    .into_any_element()
            })
            .child(axis(cx))
            .into_any_element()
    }
}

/// When this rover is expected to need the operator next.
fn forecasts(mark: &RoverMark, speed: Option<f64>) -> Vec<Forecast> {
    let mut out = Vec::new();
    if let Some(report) = mark.exploration.as_ref().filter(|r| r.running()) {
        out.push(Forecast {
            in_s: report.remaining_s,
            label: "explore budget ends".into(),
        });
    }
    if let Some(task) = mark.task.as_ref().filter(|t| t.running()) {
        out.push(Forecast {
            in_s: task.remaining_s,
            label: "task budget ends".into(),
        });
    }
    let speed = speed.filter(|s| *s >= MIN_SPEED);
    let remaining = mark
        .outdoor
        .as_ref()
        .filter(|r| r.mission_active())
        .and_then(|r| r.remaining_m)
        .or_else(|| {
            mark.goal
                .as_ref()
                .filter(|goal| goal.state == "active")
                .map(|goal| (goal.x - mark.local_x).hypot(goal.y - mark.local_y))
        });
    if let (Some(metres), Some(speed)) = (remaining, speed) {
        out.push(Forecast {
            in_s: metres / speed,
            label: "arrives · next goal".into(),
        });
    }
    out
}

/// Position on the track, 0 at 5 min ago and 1 at 10 min ahead.
fn at(t: f64, now: f64) -> f32 {
    (((t - now + TIMELINE_PAST_S) / (TIMELINE_PAST_S + TIMELINE_AHEAD_S)).clamp(0., 1.)) as f32
}

fn row_label(row: &Row, now: f64, cx: &App) -> Div {
    let (text, color): (String, Hsla) = match row.phase {
        Some(Phase::Waiting) => (
            format!("waiting {}", clock(now - row.waiting_since.unwrap_or(now))),
            rgb(WAIT_EDGE).into(),
        ),
        Some(Phase::Interaction) => (
            format!("with you {}", clock(now - row.phase_since.unwrap_or(now))),
            rgb(TALK).into(),
        ),
        Some(Phase::Neglect) => match row.forecasts.first() {
            Some(next) => (
                format!("needs you in {}", clock(next.in_s)),
                cx.theme().muted_foreground,
            ),
            None => ("running".into(), cx.theme().muted_foreground),
        },
        None => ("not reporting".into(), cx.theme().muted_foreground),
    };
    h_flex()
        .w(px(LABEL_WIDTH))
        .flex_none()
        .px_2()
        .gap_2()
        .items_baseline()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .child(format!("Rover {}", row.id)),
        )
        .child(div().text_xs().text_color(color).child(text))
}

fn track(row: &Row, now: f64) -> Div {
    let middle = (ROW_HEIGHT - BAR_HEIGHT) / 2.;
    div()
        .relative()
        .flex_1()
        .h_full()
        // A faint line every minute.
        .children((-5..=10).map(|m| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(at(now + f64::from(m) * 60., now)))
                .w(px(1.))
                .bg(rgb(GRID))
        }))
        // Now line.
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(at(now, now)))
                .w(px(1.5))
                .bg(rgb(super::palette::TEXT)),
        )
        .children(row.spans.iter().map(|span| bar(span, now, middle)))
        .children(row.forecasts.iter().map(|forecast| {
            div()
                .absolute()
                .top(px(middle - 2.))
                .left(relative(at(now + forecast.in_s, now)))
                .ml(px(-5.))
                .text_xs()
                .text_color(rgb(super::palette::MUTED))
                .whitespace_nowrap()
                .child(format!("◇ {} · +{}", forecast.label, clock(forecast.in_s)))
        }))
}

fn bar(span: &Span, now: f64, middle: f32) -> AnyElement {
    let left = at(span.start, now);
    // At least a sliver, so a short wait still shows.
    let width = (at(span.end, now) - left).max(0.002);
    let base = div().absolute().left(relative(left)).w(relative(width));
    match span.phase {
        Phase::Neglect => base
            .top(px(ROW_HEIGHT / 2. - 1.))
            .h(px(2.))
            .bg(rgb(LINE))
            .into_any_element(),
        Phase::Waiting => base
            .top(px(middle))
            .h(px(BAR_HEIGHT))
            .rounded_sm()
            .border_1()
            .border_color(rgb(WAIT_EDGE))
            .bg(rgb(WAIT_FILL))
            .into_any_element(),
        Phase::Interaction => base
            .top(px(middle))
            .h(px(BAR_HEIGHT))
            .rounded_sm()
            .bg(rgb(TALK))
            .into_any_element(),
    }
}

fn axis(cx: &App) -> Div {
    let minutes = -5..=10;
    h_flex()
        .h(px(16.))
        .child(div().w(px(LABEL_WIDTH)).flex_none())
        .child(
            div()
                .relative()
                .flex_1()
                .h_full()
                .children(minutes.map(|m| {
                    let label = match m {
                        0 => "now".to_string(),
                        m if m < 0 => format!("{m} min"),
                        m => format!("+{m}"),
                    };
                    div()
                        .absolute()
                        .top_0()
                        .left(relative(at(f64::from(m) * 60., 0.)))
                        // Keep the last label inside the panel.
                        .ml(px(if m == 10 { -22. } else { -8. }))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .when(m == 0, |d| d.font_semibold())
                        .child(label)
                })),
        )
}

fn legend(cx: &App) -> Div {
    let swatch = |fill: u32, edge: Option<u32>, height: f32| {
        div()
            .w(px(16.))
            .h(px(height))
            .rounded_sm()
            .bg(rgb(fill))
            .when_some(edge, |d, edge| d.border_1().border_color(rgb(edge)))
    };
    let item = |swatch: Div, text: &'static str| {
        h_flex().gap_1().items_center().child(swatch).child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(text),
        )
    };
    h_flex()
        .gap_3()
        .child(item(swatch(TALK, None, 8.), "interaction"))
        .child(item(
            swatch(WAIT_FILL, Some(WAIT_EDGE), 8.),
            "waiting on you",
        ))
        .child(item(swatch(LINE, None, 2.), "neglect (running alone)"))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("◇ predicted need"),
        )
}
