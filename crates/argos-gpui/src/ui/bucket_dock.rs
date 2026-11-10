//! Click a bucket's square on the map, then "Dock here": ARGOS drives the
//! selected rover toward the bucket and starts Terra's dock once the rover is
//! close enough and facing it. The drive is a one-step Score mission
//! (`argos_core::MissionRun`), so it shows in the Mission tab and Pause, Skip
//! and End mission work on it; the operator's planned mission is untouched.

use super::mission::{bucket_label, with_view};
use crate::Dashboard;

use argos_core::{
    DOCK_START_M, MissionPlan, MissionRun, MissionStep, geodesic_metres, in_dock_window,
    mission_block,
};
use argos_gpui::RoverMark;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt, ThemeStyled,
    button::{Button, ButtonVariants},
};
use gpui_kit::{prelude::*, *};

/// How near the pointer must be to a bucket square, pixels (the square is 16 px).
const HIT_PX: f32 = 12.;
/// Another rover's bucket is the selected rover's bucket when their map
/// positions are this close, metres (Terra merges sightings within 1 m).
const SAME_BUCKET_M: f64 = 1.0;

/// The bucket square the operator clicked.
#[derive(Clone, Debug)]
pub(crate) struct BucketPick {
    /// Rover whose object map holds the clicked bucket.
    pub(crate) owner: u64,
    pub(crate) object_id: u32,
    pub(crate) color: Option<String>,
    pub(crate) label: String,
    pub(crate) latitude: f64,
    pub(crate) longitude: f64,
}

/// What "Dock here" would do for the selected rover.
#[derive(Clone, Debug, PartialEq)]
enum DockPlan {
    /// Already within range and facing the bucket: start the dock now.
    DockNow { rover: u64, object_id: u32 },
    /// Drive toward the bucket (L2/L3 goal), then dock.
    Drive {
        rover: u64,
        object_id: u32,
        label: String,
        x: f64,
        y: f64,
    },
}

impl Dashboard {
    /// The bucket square under a window position, nearest first.
    pub(super) fn bucket_at(&self, position: Point<Pixels>, cx: &App) -> Option<BucketPick> {
        let map = self.view.read(cx);
        let mut best: Option<(f32, BucketPick)> = None;
        for mark in &self.marks {
            for object in &mark.objects {
                if object.object.class != "bucket" {
                    continue;
                }
                let Some((latitude, longitude)) = object.map_position else {
                    continue;
                };
                let Some(at) = map.project(latitude, longitude) else {
                    continue;
                };
                let gap = f32::from(at.x - position.x).hypot(f32::from(at.y - position.y));
                if gap <= HIT_PX && best.as_ref().is_none_or(|(seen, _)| gap < *seen) {
                    best = Some((
                        gap,
                        BucketPick {
                            owner: mark.id,
                            object_id: object.object.id,
                            color: object.object.color.clone(),
                            label: bucket_label(&object.object),
                            latitude,
                            longitude,
                        },
                    ));
                }
            }
        }
        best.map(|(_, pick)| pick)
    }

    /// A click on a bucket square opens its popup. Returns false when the
    /// click missed every bucket. While placing mission steps, a click on one
    /// of the selected rover's buckets adds a Score step instead.
    pub(super) fn click_bucket(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(pick) = self.bucket_at(position, cx) else {
            return false;
        };
        if self.missions.placing.is_some()
            && let Some(mark) = self.selected_mark().cloned()
        {
            match own_bucket(&mark, &pick) {
                Some(object) => {
                    let label = bucket_label(&object.object);
                    let step = MissionStep::score(
                        object.object.id,
                        label.clone(),
                        object.object.x,
                        object.object.y,
                    );
                    let plan = self.missions.plans.entry(mark.id).or_default();
                    self.note = match plan.push(step) {
                        Ok(()) => format!("Step {} added: Score at {label}", plan.steps.len()),
                        Err(reason) => reason.into(),
                    };
                }
                None => {
                    self.note = format!("Rover {} has not seen {} yet", mark.id, pick.label);
                }
            }
            return true;
        }
        self.drag = None;
        self.note = format!("{} · Dock here sends the selected rover to it", pick.label);
        self.bucket_pick = Some(pick);
        true
    }

    /// What "Dock here" would do now, or why it cannot.
    fn bucket_dock_plan(&self, pick: &BucketPick) -> Result<DockPlan, String> {
        let mark = self
            .selected_mark()
            .ok_or_else(|| "Select a rover to dock it here".to_string())?;
        let id = mark.id;
        let object = own_bucket(mark, pick).ok_or_else(|| {
            format!(
                "Rover {id} has not seen this bucket yet · drive it closer so its camera finds it"
            )
        })?;
        if !self.claim.held(self.now) {
            return Err("Control the fleet to dock".into());
        }
        if self.driving {
            return Err("Leave drive before docking".into());
        }
        if self.missions.runs.get(&id).is_some_and(MissionRun::active) {
            return Err(format!("End rover {id}'s mission first"));
        }
        let (bx, by) = (object.object.x, object.object.y);
        // A mission can drive there (L2/L3, pose fresh, local source).
        if with_view(mark, mission_block).is_none() {
            return Ok(DockPlan::Drive {
                rover: id,
                object_id: object.object.id,
                label: bucket_label(&object.object),
                x: bx,
                y: by,
            });
        }
        // Any level may dock on a bucket it already faces up close, as the
        // Dock button in the rover panel does.
        if mark.safety != "emergency_stop"
            && in_dock_window(mark.local_x, mark.local_y, mark.yaw, bx, by)
        {
            return Ok(DockPlan::DockNow {
                rover: id,
                object_id: object.object.id,
            });
        }
        Err(with_view(mark, mission_block).unwrap_or_default())
    }

    /// The popup's "Dock here".
    pub(super) fn dock_at_bucket(&mut self) {
        let Some(pick) = self.bucket_pick.take() else {
            return;
        };
        let plan = match self.bucket_dock_plan(&pick) {
            Ok(plan) => plan,
            Err(reason) => {
                self.note = reason;
                self.bucket_pick = Some(pick);
                return;
            }
        };
        match plan {
            DockPlan::DockNow { rover, object_id } => {
                self.feed.start_dock(rover, object_id);
                self.feed.record(serde_json::json!({
                    "kind": "bucket_dock",
                    "rover_id": rover,
                    "object_id": object_id,
                    "mode": "dock_now",
                }));
                self.note = format!("Rover {rover} docking on {}", pick.label);
            }
            DockPlan::Drive {
                rover,
                object_id,
                label,
                x,
                y,
            } => {
                let Some(mark) = self.marks.iter().find(|mark| mark.id == rover).cloned() else {
                    return;
                };
                let plan = MissionPlan {
                    steps: vec![MissionStep::score(object_id, label.clone(), x, y)],
                    repeat: false,
                };
                let now = self.now;
                match with_view(&mark, |view| MissionRun::start(plan, view, now)) {
                    Ok((run, tick)) => {
                        self.missions.placing = None;
                        self.feed.record(serde_json::json!({
                            "kind": "bucket_dock",
                            "rover_id": rover,
                            "object_id": object_id,
                            "mode": "drive_then_dock",
                            "plan": run.plan,
                        }));
                        self.note = format!(
                            "Rover {rover} driving to {label} · docks within {DOCK_START_M:.2} m, facing it"
                        );
                        self.missions.runs.insert(rover, run);
                        self.apply_mission(rover, tick);
                    }
                    Err(reason) => self.note = reason,
                }
            }
        }
    }

    pub(super) fn close_bucket_pick(&mut self) {
        self.bucket_pick = None;
    }
}

/// The selected rover's own object for the clicked bucket: the same object
/// when it is the rover's own, else its bucket of the same colour nearest the
/// clicked one on the map, within [`SAME_BUCKET_M`].
fn own_bucket<'a>(mark: &'a RoverMark, pick: &BucketPick) -> Option<&'a argos_gpui::ObjectMark> {
    let buckets = mark
        .objects
        .iter()
        .filter(|object| object.object.class == "bucket");
    if pick.owner == mark.id {
        return buckets
            .into_iter()
            .find(|object| object.object.id == pick.object_id);
    }
    buckets
        .filter(|object| {
            pick.color.is_none()
                || object.object.color.is_none()
                || object.object.color == pick.color
        })
        .filter_map(|object| {
            let (latitude, longitude) = object.map_position?;
            let gap = geodesic_metres(latitude, longitude, pick.latitude, pick.longitude);
            (gap <= SAME_BUCKET_M).then_some((gap, object))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, object)| object)
}

/// The popup over a clicked bucket: what Dock here will do, or why not.
pub(super) fn bucket_popup(
    dashboard: &Dashboard,
    pick: &BucketPick,
    at: Point<Pixels>,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let plan = dashboard.bucket_dock_plan(pick);
    let selected = dashboard.selected_mark();
    let distance = selected.and_then(|mark| {
        let object = own_bucket(mark, pick)?;
        Some((object.object.x - mark.local_x).hypot(object.object.y - mark.local_y))
    });
    let detail = match (&plan, distance) {
        (Ok(DockPlan::DockNow { .. }), Some(d)) => {
            format!("{d:.1} m away and facing it · docks now")
        }
        (Ok(DockPlan::Drive { .. }), Some(d)) => {
            format!("{d:.1} m away · drives up, docks within {DOCK_START_M:.2} m")
        }
        (Ok(_), None) => String::new(),
        (Err(reason), _) => reason.clone(),
    };
    let rover = selected.map(|mark| mark.id);
    let title = match rover {
        Some(id) => format!("Dock rover {id} at {}?", pick.label),
        None => pick.label.clone(),
    };
    let dumps = "The dump still waits for your approval at L0–L3.";
    div().absolute().left(at.x).top(at.y).size_0().child(
        v_flex()
            .id("bucket-dock")
            .absolute()
            .bottom_4()
            .left(px(-120.))
            .w(px(240.))
            .p_2()
            .gap_2()
            .popover_style(cx)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().font_semibold().child(title))
                    .child(
                        div()
                            .text_xs()
                            .text_color(if plan.is_ok() {
                                cx.theme().muted_foreground
                            } else {
                                cx.theme().warning
                            })
                            .child(detail),
                    )
                    .when(plan.is_ok(), |col| {
                        col.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(dumps),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("bucket-dock-cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_bucket_pick();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("bucket-dock-go")
                            .primary()
                            .small()
                            .label("Dock here")
                            .disabled(plan.is_err())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dock_at_bucket();
                                cx.notify();
                            })),
                    ),
            ),
    )
}
