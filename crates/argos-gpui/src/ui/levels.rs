//! Autonomy level buttons and the study condition lock.

use super::widgets::Picked;
use super::widgets::caution;
use super::widgets::hint;
use super::widgets::section;
use crate::Dashboard;

use argos_core::{OPERATOR_LEVELS, STUDY_LEVELS, level_label, level_permitted};
use argos_gpui::RoverMark;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonGroup},
};
use gpui_kit::prelude::*;

/// One row of the autonomy picker over `OPERATOR_LEVELS[range]`; the
/// requested level is selected.
pub(super) fn level_group(
    id: u64,
    requested: Option<usize>,
    lock: Option<String>,
    range: std::ops::Range<usize>,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let start = range.start;
    ButtonGroup::new((
        "level",
        id.saturating_mul(2).saturating_add(u64::from(start > 0)),
    ))
    .outline()
    .small()
    .children(
        OPERATOR_LEVELS[range]
            .iter()
            .enumerate()
            .map(|(offset, (wire, label))| {
                let index = start + offset;
                let permitted = level_permitted(lock.as_deref(), wire);
                Button::new(("level", index as u64))
                    .label(*label)
                    .picked(requested == Some(index))
                    .disabled(!permitted)
                    .when(!permitted, |button| {
                        button.tooltip(
                            "The study condition fixes the level. Change it in Study & tasks.",
                        )
                    })
            }),
    )
    .on_click(cx.listener(move |this, clicks: &Vec<usize>, _, cx| {
        if this.selected == Some(id)
            && let Some((wire, _)) = clicks
                .first()
                .and_then(|offset| OPERATOR_LEVELS.get(start + offset))
        {
            let lock = this
                .marks
                .iter()
                .find(|mark| mark.id == id)
                .and_then(|mark| this.study_lock(mark));
            if !level_permitted(lock, wire) {
                this.note = format!("The study condition locks {}", level_label(wire));
            } else if *wire == "explore" {
                this.start_explore(id);
            } else if *wire == "task" {
                this.start_task(id, cx);
            } else {
                this.feed.publish_level(id, wire);
                this.note = format!("Sending {}", level_label(wire));
            }
        }
        cx.notify();
    }))
}

/// Outdoor GPS is the source but this Terra build cannot run `level` with it.
pub(super) fn gps_blocks(mark: &RoverMark, level: &str) -> bool {
    mark.outdoor_gps
        && !mark
            .outdoor
            .as_ref()
            .is_some_and(|report| report.accepts_goal_at(level))
}

/// How many times the window sends a study condition to a rover that does not
/// report it before warning that the rover refused it.
pub(super) const STUDY_SENDS: u32 = 5;

/// Study condition for the whole fleet: off, or one fixed level. Terra
/// enforces it, so other levels stay locked even from the phone.
pub(super) fn study_section(
    dashboard: &Dashboard,
    mark: &RoverMark,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let current = match dashboard.study {
        Some(choice) => choice,
        None => mark.assigned_level.as_deref().and_then(|level| {
            STUDY_LEVELS
                .iter()
                .find(|(wire, _)| *wire == level)
                .map(|(wire, _)| *wire)
        }),
    };
    let selected = current.map_or(0, |level| {
        STUDY_LEVELS
            .iter()
            .position(|(wire, _)| *wire == level)
            .map_or(0, |index| index + 1)
    });
    let pending = dashboard
        .study
        .is_some_and(|want| mark.assigned_level.as_deref() != want);
    section("Study condition · all rovers", cx)
        .child(
            ButtonGroup::new("study")
                .outline()
                .small()
                .child(Button::new(("study", 0u64)).label("Off").picked(selected == 0))
                .children(STUDY_LEVELS.iter().enumerate().map(|(index, (_, short))| {
                    Button::new(("study", index as u64 + 1))
                        .label(*short)
                        .picked(selected == index + 1)
                }))
                .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                    if let Some(index) = clicks.first() {
                        let level = index
                            .checked_sub(1)
                            .and_then(|index| STUDY_LEVELS.get(index))
                            .map(|(wire, _)| *wire);
                        this.set_study(level);
                    }
                    cx.notify();
                })),
        )
        .map(|section| {
            if dashboard.study_refused(mark) {
                section.child(caution(
                    "This rover did not take the study condition · update Terra on it",
                    cx,
                ))
            } else if pending {
                section.child(hint("Sending the study condition to this rover…", cx))
            } else {
                section.child(hint(
                    "Fixes every rover at one level for a trial. Terra rejects the other levels; L0 takeover and Stop stay available. ARGOS and Terra both log it.",
                    cx,
                ))
            }
        })
}
