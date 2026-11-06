//! Keep the window in step with the fleet: startup, the 100 ms sync from
//! `FleetFeed`, 3D rover markers, the preview fixture, and fleet switching.

use super::agent;
use super::interaction::{geographic_gate, gps_point};
use super::levels::STUDY_SENDS;
use crate::{ActiveDrag, Dashboard};

use argos_core::{
    ControlClaim, DragAnchor, DragGate, DragSpace, GoalDrag, PlanarPoint, STUDY_LEVELS, drive_block,
};
use argos_gpui::console::{self, InspectorTab};
use argos_gpui::{
    DashboardConfig, FleetFeed, FpvController, LaunchMode, RoverMark, model_position,
    reframing_target,
};
use gpui_kit::component::input::InputState;
use gpui_kit::{prelude::*, *};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use terra_map::{Color, GeoPosition, Map, MapEvent, MapMode, MapView};

// Measured 35-inch square footprint, including wheels. Asset is Z-up after loading.
const ROVER_AXIS_SCALE: [f64; 3] = [0.61 / 2.45, 0.61 / 2.5, 0.61 / 2.5];

impl Dashboard {
    pub(crate) fn new(
        map: Map,
        feed: FleetFeed,
        fpv: FpvController,
        maps_notice: Option<String>,
        fleet_mode: LaunchMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let model = argos_gpui::robot_model();
        let task_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Task for the rover, e.g. find the orange bucket behind the wall")
        });
        let task_reply =
            cx.new(|cx| InputState::new(window, cx).placeholder("Answer or extra guidance"));
        let view = cx.new(|cx| MapView::new(map.clone(), cx));
        let focus = cx.focus_handle();
        let poll = cx.spawn(async |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        while let Some(event) = this.map.poll_event() {
                            if let MapEvent::ObjectLoadFailed { message, .. } = event {
                                this.note = message;
                            }
                        }
                        for retired in this.fpv.take_retired() {
                            cx.drop_image(retired, None);
                        }
                        this.sync(cx);
                        this.serve_agents();
                        if this.close_preview {
                            this.close_preview = false;
                            this.view.update(cx, |map, _| map.frame_near());
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let initial = feed.picture();
        Self {
            map,
            view,
            feed,
            focus,
            model,
            objects: BTreeMap::new(),
            goals: BTreeMap::new(),
            framed: BTreeSet::new(),
            framed_center: None,
            marks: Vec::new(),
            presence: Vec::new(),
            selected: None,
            selection_hold: argos_gpui::SelectionHold::default(),
            map_mode: MapMode::default(),
            drag: None,
            press: None,
            field_frame: Arc::new(Mutex::new(None)),
            inspector_frame: Arc::new(Mutex::new(None)),
            show_occupancy: true,
            muted_map: false,
            inspector_tab: InspectorTab::default(),
            driving: false,
            held: BTreeSet::new(),
            link: initial.link,
            detail: initial.detail,
            command: initial.command,
            note: String::new(),
            maps_notice,
            preview: std::env::var("ARGOS_PREVIEW_DRAG").ok().as_deref() == Some("1"),
            preview_done: false,
            preview_claim: false,
            close_preview: false,
            started: Instant::now(),
            now: 0.,
            claim: ControlClaim::new(),
            latched: BTreeMap::new(),
            rover_stops: BTreeMap::new(),
            explore_budget: 1,
            task_input,
            task_reply,
            task_budget: 1,
            task_agent: None,
            fpv,
            study: None,
            study_sends: BTreeMap::new(),
            fleet_mode,
            agent: agent::AgentDesk::start(),
            attention: super::attention::AttentionLog::start(),
            missions: Default::default(),
            _poll: poll,
        }
    }
    fn sync(&mut self, cx: &Context<Self>) {
        let picture = self.feed.picture();
        self.link.clone_from(&picture.link);
        self.detail.clone_from(&picture.detail);
        if !picture.command.is_empty() {
            self.command.clone_from(&picture.command);
        }
        for change in console::reporting_changes(&self.marks, &picture.marks) {
            self.feed.record(change);
        }
        self.marks = picture.marks;
        self.presence = picture.presence;
        let now = self.started.elapsed().as_secs_f64();
        if let Some(id) = self
            .selected
            .filter(|id| !self.marks.iter().any(|mark| mark.id == *id))
        {
            self.stop_drive();
            self.selected = None;
            self.drag = None;
            self.selection_hold.lose(id, now);
        } else if self.selected.is_none()
            && let Some(id) = self
                .selection_hold
                .restore(now, |id| self.marks.iter().any(|mark| mark.id == id))
        {
            // Back from a dropout: reopen its panel. Driving stays stopped.
            self.selected = Some(id);
            self.note = format!("Rover {id} is reporting again");
        }
        // Only the selected rover's grid is drawn, so only it is subscribed.
        // A rover in a dropout stays subscribed so its map is kept.
        self.feed
            .focus(self.selection_hold.focus(self.selected, now));
        if self.driving
            && let Some(id) = self.selected
            && !self.held.is_empty()
        {
            self.feed.refresh_drive(id);
        }
        if self.driving {
            let reason = self
                .selected
                .and_then(|id| self.marks.iter().find(|mark| mark.id == id))
                .and_then(|mark| drive_block(&mark.hardware))
                .map(str::to_owned);
            if let Some(reason) = reason {
                self.stop_drive();
                self.note = reason;
            }
        }
        self.now = self.started.elapsed().as_secs_f64();
        self.claim.renew(self.now);
        self.sync_missions();
        let seen: BTreeSet<u64> = self.marks.iter().map(|mark| mark.id).collect();
        self.latched.retain(|id, _| seen.contains(id));
        self.log_rover_stops();
        self.sync_markers(&seen, false, self.view.read(cx).ground_altitude());
        self.sync_markers(&seen, true, self.view.read(cx).ground_altitude());
        if seen.is_empty() {
            self.framed.clear();
            self.framed_center = None;
        } else if self.drag.is_none()
            && !self.driving
            && let Some((ids, latitude, longitude)) =
                reframing_target(&self.framed, self.framed_center, &self.marks)
            && self.map.show_lat_lon(latitude, longitude).is_ok()
        {
            self.framed = ids;
            self.framed_center = Some((latitude, longitude));
        }
        self.sync_study();
        self.seed_preview();
    }
    /// Keep every rover on this window's study condition. Terra forgets it on
    /// restart, so a rover that reports another condition is sent it again,
    /// at most every 2 s and [`STUDY_SENDS`] times per change.
    pub(super) fn sync_study(&mut self) {
        let Some(want) = self.study else {
            return;
        };
        let ids: Vec<(u64, bool)> = self
            .marks
            .iter()
            .map(|mark| (mark.id, mark.assigned_level.as_deref() == want))
            .collect();
        self.study_sends
            .retain(|id, _| ids.iter().any(|(seen, _)| seen == id));
        for (id, matches) in ids {
            if matches {
                self.study_sends.remove(&id);
                continue;
            }
            let (at, count) = self.study_sends.entry(id).or_insert((f64::NEG_INFINITY, 0));
            if *count < STUDY_SENDS && self.now - *at >= 2. {
                (*at, *count) = (self.now, *count + 1);
                self.feed.publish_assignment(id, want);
            }
        }
    }
    pub(super) fn export_session(&mut self) {
        let label = match self.study {
            Some(Some(level)) => STUDY_LEVELS
                .iter()
                .find(|(wire, _)| *wire == level)
                .map_or("study", |(_, short)| *short),
            _ => "free",
        };
        self.note = match self.feed.export_session(label) {
            Ok(path) => format!("Session log exported to {}", path.display()),
            Err(error) => error,
        };
    }
    fn seed_preview(&mut self) {
        if !self.preview || self.preview_done || self.drag.is_some() {
            return;
        }
        let Some(mark) = self
            .marks
            .iter()
            .find(|mark| mark.status == "online")
            .cloned()
        else {
            return;
        };
        self.selected = Some(mark.id);
        if !self.preview_claim {
            self.claim.take(self.now);
            self.preview_claim = true;
        }
        if !mark.outdoor_gps
            && (mark.raw_gps.is_some() || mark.outdoor.is_some())
            && mark
                .outdoor
                .as_ref()
                .is_none_or(|report| report.source != "local")
        {
            return;
        }
        let geographic = mark.outdoor_gps;
        let level = mark
            .effective_level
            .clone()
            .or_else(|| mark.requested_level.clone());
        let gate = if geographic {
            geographic_gate(&mark, &self.link, true)
        } else {
            DragGate {
                selected: true,
                link_healthy: self.link == "healthy",
                pose_fresh: true,
                degraded: mark.degraded,
                level: level.as_deref(),
                geographic: false,
                outdoor: mark.outdoor_gps,
                admission: None,
            }
        };
        let started = if geographic {
            let rover = gps_point(&mark);
            GoalDrag::start(
                mark.id,
                DragAnchor::Rover,
                DragSpace::Geographic,
                rover,
                rover,
                Some(mark.arrival_radius),
                gate,
            )
        } else {
            let rover = PlanarPoint {
                x: mark.local_x,
                y: mark.local_y,
            };
            GoalDrag::start(
                mark.id,
                DragAnchor::Rover,
                DragSpace::Local,
                rover,
                rover,
                None,
                gate,
            )
        };
        let mut goal = match started {
            Ok(goal) => goal,
            Err(reason) => {
                if geographic {
                    self.note = reason.into();
                }
                return;
            }
        };
        if geographic {
            let rover = gps_point(&mark);
            goal.move_to(PlanarPoint {
                x: rover.x + 0.00025,
                y: rover.y,
            });
        } else {
            goal.move_to(PlanarPoint {
                x: mark.local_x + 2.,
                y: mark.local_y + 1.,
            });
        }
        self.note = match goal.arrival_radius_m {
            Some(radius) => format!(
                "{:.1} m GPS · arrival radius {radius:.0} m",
                goal.distance_m()
            ),
            None => format!("{:.1} m", goal.distance_m()),
        };
        self.drag = Some(ActiveDrag {
            goal,
            from_map: geographic,
        });
        self.preview_done = true;
        self.close_preview = true;
    }
    fn sync_markers(&mut self, seen: &BTreeSet<u64>, goals: bool, altitude: f64) {
        let objects = if goals {
            &mut self.goals
        } else {
            &mut self.objects
        };
        let gone: Vec<u64> = objects
            .keys()
            .copied()
            .filter(|id| {
                !seen.contains(id)
                    || (goals
                        && self
                            .marks
                            .iter()
                            .find(|mark| mark.id == *id)
                            .is_none_or(|mark| {
                                mark.goal
                                    .as_ref()
                                    .is_none_or(|goal| goal.latitude.is_none())
                            }))
            })
            .collect();
        for id in gone {
            if let Some(object) = objects.remove(&id) {
                let _ = self.map.remove_object(object);
            }
        }
        for mark in &self.marks {
            let (latitude, longitude, scale, tint) = if goals {
                let Some(goal) = mark.goal.as_ref() else {
                    continue;
                };
                let (Some(latitude), Some(longitude)) = (goal.latitude, goal.longitude) else {
                    continue;
                };
                (latitude, longitude, 1., Some(Color::rgb(40, 200, 140)))
            } else {
                let preview = self
                    .drag
                    .as_ref()
                    .filter(|active| active.from_map)
                    .map(|active| &active.goal);
                let (latitude, longitude) = model_position(
                    mark.id,
                    (mark.latitude, mark.longitude),
                    mark.overlay_frame,
                    preview,
                );
                let picked = preview
                    .is_some_and(|drag| drag.id == mark.id && drag.anchor == DragAnchor::Rover);
                (
                    latitude,
                    longitude,
                    1.,
                    if picked {
                        Some(Color::rgb(255, 160, 50))
                    } else if self.selected == Some(mark.id) && self.claim.held(self.now) {
                        Some(Color::rgb(61, 204, 154))
                    } else {
                        (mark.status != "online").then_some(Color::rgb(255, 160, 50))
                    },
                )
            };
            let altitude = self
                .map
                .surface_altitude(latitude, longitude)
                .unwrap_or(altitude);
            let position = GeoPosition::new(latitude, longitude, altitude);
            let object = if let Some(object) = objects.get(&mark.id).copied() {
                if self.map.set_object_position(object, position).is_err() {
                    continue;
                }
                object
            } else {
                match self.map.add_3d_obj(position, &self.model) {
                    Ok(object) => {
                        let _ = self.map.set_object_scale(object, scale);
                        let _ = self.map.set_object_axis_scale(object, ROVER_AXIS_SCALE);
                        objects.insert(mark.id, object);
                        object
                    }
                    Err(error) => {
                        self.note = error.to_string();
                        continue;
                    }
                }
            };
            if !goals {
                let _ = self.map.set_object_heading(object, mark.heading);
            }
            if let Some(color) = tint {
                let _ = self.map.illuminate_3d_object(object, color);
            } else {
                let _ = self.map.clear_illumination(object);
            }
        }
    }
    /// Field position of a proposed destination, so its popup follows the
    /// point when the camera moves.
    pub(super) fn proposal_at(&self, active: &ActiveDrag, cx: &App) -> Option<Point<Pixels>> {
        let goal = &active.goal;
        let (latitude, longitude) = match goal.space {
            DragSpace::Geographic => (goal.cursor.x, goal.cursor.y),
            DragSpace::Local => self
                .marks
                .iter()
                .find(|mark| mark.id == goal.id)?
                .overlay_frame?
                .geographic(goal.cursor.x, goal.cursor.y)
                .ok()?,
        };
        self.view.read(cx).project_local(latitude, longitude)
    }
    pub(super) fn selected_mark(&self) -> Option<&RoverMark> {
        self.selected
            .and_then(|id| self.marks.iter().find(|mark| mark.id == id))
    }
    /// Reconnect this window to the simulator or the real fleet. Drive input
    /// is zeroed and control released first. Rovers keep whatever level and
    /// goal they had; this only changes which bus the window talks to.
    pub(super) fn switch_fleet(&mut self, mode: LaunchMode, window: &mut Window) {
        if mode == self.fleet_mode {
            return;
        }
        let config = match DashboardConfig::from_env_mode(mode) {
            Ok(config) => config,
            Err(error) => {
                self.note = format!("Stayed on {}: {error}", self.fleet_mode.label());
                return;
            }
        };
        self.end_agent_drive("fleet switched");
        let running: Vec<u64> = self.missions.runs.keys().copied().collect();
        for id in running {
            self.halt_mission(id, "fleet switched");
        }
        self.missions.runs.clear();
        self.missions.placing = None;
        self.stop_drive();
        self.drag = None;
        self.selected = None;
        self.selection_hold.clear();
        self.claim.release();
        self.latched.clear();
        self.rover_stops.clear();
        self.framed.clear();
        self.framed_center = None;
        self.feed.shutdown();
        self.feed = FleetFeed::start(config.clone());
        // Dropping the old controller ends its video call.
        self.fpv = FpvController::start(config);
        self.fleet_mode = mode;
        // The next sync drops markers for rovers the new fleet does not have.
        self.marks.clear();
        self.presence.clear();
        let picture = self.feed.picture();
        self.link = picture.link;
        self.detail = picture.detail;
        self.command.clear();
        self.note = format!("Switched to {} · {}", mode.label(), self.detail);
        window.set_window_title(mode.title());
    }
    pub(super) fn set_map_mode(&mut self, mode: MapMode) {
        self.map_mode = mode;
        self.map.set_mode(mode);
    }
    /// Log each rover's new `emergency_stop`, and each change of its reason,
    /// once, naming whether this
    /// window sent it and the rover's reason, so the session log keeps the
    /// cause the callout showed.
    fn log_rover_stops(&mut self) {
        let mut now_stopped = BTreeMap::new();
        for mark in &self.marks {
            if mark.safety != "emergency_stop" {
                continue;
            }
            now_stopped.insert(mark.id, mark.autonomy_reason.clone());
            if self.rover_stops.get(&mark.id) == Some(&mark.autonomy_reason) {
                continue;
            }
            let sent = self.latched.get(&mark.id);
            self.feed.record(serde_json::json!({
                "kind": "stop_observed",
                "rover_id": mark.id,
                "sent_here": sent.is_some(),
                "origin": sent.map(|(origin, _)| origin.name()),
                "reason": mark.autonomy_reason,
                "label": self.stop_cause(mark).map(|cause| cause.label()),
            }));
        }
        self.rover_stops = now_stopped;
    }
    /// While the operator types a task, letter keys and Space are text, not drive or stop.
    pub(super) fn typing(&self, window: &Window, cx: &App) -> bool {
        self.task_input.read(cx).focus_handle(cx).is_focused(window)
            || self.task_reply.read(cx).focus_handle(cx).is_focused(window)
    }
}
