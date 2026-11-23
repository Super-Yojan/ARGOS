//! The bar across the top: fleet summary on the left, STOP ALL on the right.
//!
//! Anything added with `.child(..)` sits just left of STOP ALL.

use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::widgets::{ButtonLook, button, mono};
use crate::fleet::{Fleet, LinkState, help_queue};
use crate::theme;

#[derive(IntoElement)]
pub struct TopBar {
    fleet: Fleet,
    /// Set while the operator drives a rover by hand.
    driving: Option<SharedString>,
    children: Vec<AnyElement>,
}

impl TopBar {
    pub fn new(fleet: Fleet) -> Self {
        Self {
            fleet,
            driving: None,
            children: Vec::new(),
        }
    }

    /// Turn the bar blue and show `banner` instead of the fleet summary, so
    /// it is obvious that the keyboard is driving a rover.
    pub fn driving(mut self, banner: impl Into<SharedString>) -> Self {
        self.driving = Some(banner.into());
        self
    }
}

impl ParentElement for TopBar {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for TopBar {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let driving = self.driving.is_some();
        let summary = match self.driving {
            Some(banner) => div()
                .font_family(theme::FONT_HEADING)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(16.0))
                .text_color(rgb(theme::BLUE_PALE))
                .child(banner)
                .into_any_element(),
            None => fleet_summary(&self.fleet).into_any_element(),
        };

        let fleet = self.fleet;
        let stop_all = button("stop-all", "STOP ALL", ButtonLook::Stop)
            .min_h(px(30.0))
            .px(px(14.0))
            .font_family(theme::FONT_HEADING)
            .text_size(px(12.5))
            .on_click(move |_, _, cx| {
                fleet.stop_all();
                cx.refresh_windows();
            });

        h_flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap_3()
            .px_3()
            .py(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .bg(rgb(theme::PANEL))
            .border_color(rgb(theme::BORDER))
            .when(driving, |bar| {
                bar.bg(rgb(theme::BLUE_BG)).border_color(rgb(theme::BLUE))
            })
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .font_family(theme::FONT_HEADING)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(15.0))
                            .text_color(rgb(theme::TEXT))
                            .child("ARGOS"),
                    )
                    .child(
                        mono("/")
                            .text_color(rgb(theme::BORDER)),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(13.0))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child("Ops Deck"),
                    ),
            )
            .child(summary)
            .child(div().flex_1())
            .children(self.children)
            .child(stop_all)
    }
}

/// "tcp/… · 6 rovers   2 need you   4 running".
fn fleet_summary(fleet: &Fleet) -> Div {
    let rovers = fleet.rovers();
    let need_you = help_queue(&rovers).len();
    let on_their_own = rovers.len() - need_you;

    let (link, link_color) = match fleet.link_state() {
        LinkState::Connected => (
            format!("{} · {}", fleet.endpoint(), count(rovers.len(), "rover")),
            theme::TEXT_MUTED,
        ),
        LinkState::Connecting => (
            format!("Connecting to {}…", fleet.endpoint()),
            theme::ORANGE_TEXT,
        ),
        LinkState::ConfigError(error) => (format!("Config error: {error}"), theme::RED.accent),
    };

    h_flex()
        .gap_3()
        .items_center()
        .child(
            mono(link)
                .px(px(10.0))
                .py(px(3.0))
                .rounded_full()
                .bg(rgb(theme::PANEL_MUTED))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .text_color(rgb(link_color)),
        )
        .when(need_you > 0, |row| {
            let verb = if need_you == 1 { "needs" } else { "need" };
            row.child(
                mono(format!("{need_you} {verb} you"))
                    .text_size(px(13.0))
                    .text_color(rgb(theme::ORANGE_TEXT)),
            )
        })
        .when(on_their_own > 0, |row| {
            row.child(
                mono(format!("{on_their_own} on their own"))
                    .text_size(px(13.0))
                    .text_color(rgb(theme::GREEN)),
            )
        })
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
