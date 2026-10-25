//! ARGOS operator dashboard.
//!
//! The camera starts at `ARGOS_MAP_START_LAT` / `ARGOS_MAP_START_LON` until rover
//! positions arrive, then recenters on the fleet. `MAPS_DEMO=1` still draws the
//! offline fixture. Clicking a rover selects it and takes local control.
//! With a rover selected, clicking the map proposes a destination there and
//! opens a small popup at the click; Send goal publishes it. Esc, Cancel, or a
//! point on the rover sends nothing. An acknowledged outdoor GPS rover uses the
//! same click: the ghost shows the arrival radius and GPS distance, and Send
//! goal publishes the WGS84 point as the explicit operator action.

mod interaction;
mod local_map;
mod map_overlay;
mod palette;

use interaction::{geographic_gate, goal_geo, gps_point};
use map_overlay::map_paint;

use argos_core::{
    APP_UPDATE_REQUIRED, ControlClaim, DragAnchor, DragGate, DragSpace, EXPLORE_BUDGETS_MIN,
    GoalDrag, HEADING_MAX_AGE_S, HEADING_MAX_UNCERTAINTY_DEG, OPERATOR_LEVELS, OutdoorReport,
    PlanarPoint, drag_disabled_reason, drive_block, explore_refusal, level_label, phase_label,
};
use argos_gpui::console::{self, CalloutTag, InspectorTab};
use argos_gpui::{
    DashboardConfig, FleetFeed, FpvController, FpvPhase, LaunchMode, RoverMark, RoverPresence,
    model_position, position_label, reframing_target,
};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Selectable, Sizable, StyledExt, Theme, ThemeMode, ThemeStyled, TitleBar,
    button::{Button, ButtonGroup, ButtonVariants},
    description_list::{DescriptionItem, DescriptionList},
    scroll::ScrollableElement,
    tab::{Tab, TabBar},
    tag::Tag,
    tooltip::Tooltip,
};
use gpui_kit::{prelude::*, *};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use terra_map::{Color, GeoPosition, Map, MapConfig, MapEvent, MapMode, MapView, ObjectId};

// Measured 35-inch square footprint, including wheels. Asset is Z-up after loading.
const ROVER_AXIS_SCALE: [f64; 3] = [0.889 / 2.45, 0.889 / 2.5, 0.889 / 2.5];

struct ActiveDrag {
    goal: GoalDrag,
    from_map: bool,
}

struct Dashboard {
    map: Map,
    view: Entity<MapView>,
    feed: FleetFeed,
    focus: FocusHandle,
    model: PathBuf,
    objects: BTreeMap<u64, ObjectId>,
    goals: BTreeMap<u64, ObjectId>,
    framed: BTreeSet<u64>,
    framed_center: Option<(f64, f64)>,
    marks: Vec<RoverMark>,
    presence: Vec<RoverPresence>,
    selected: Option<u64>,
    map_mode: MapMode,
    drag: Option<ActiveDrag>,
    /// Where the left button went down on the map, to tell a click from an
    /// orbit or pan.
    press: Option<Point<Pixels>>,
    /// The map area and the inspector, measured at prepaint for rover card
    /// placement.
    field_frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
    inspector_frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
    show_occupancy: bool,
    inspector_tab: InspectorTab,
    driving: bool,
    held: BTreeSet<String>,
    link: String,
    detail: String,
    command: String,
    note: String,
    maps_notice: Option<String>,
    preview: bool,
    preview_done: bool,
    preview_claim: bool,
    close_preview: bool,
    started: Instant,
    now: f64,
    claim: ControlClaim,
    latched: BTreeSet<u64>,
    /// Index into `EXPLORE_BUDGETS_MIN` for the next L4 Explore request.
    explore_budget: usize,
    fpv: FpvController,
    /// Which fleet `feed` and `fpv` are connected to. The console toggle
    /// replaces both.
    fleet_mode: LaunchMode,
    _poll: Task<()>,
}
impl Dashboard {
    fn new(
        map: Map,
        feed: FleetFeed,
        fpv: FpvController,
        maps_notice: Option<String>,
        fleet_mode: LaunchMode,
        cx: &mut Context<Self>,
    ) -> Self {
        let model =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../terra-map/src/assets/robot.glb");
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
            map_mode: MapMode::default(),
            drag: None,
            press: None,
            field_frame: Arc::new(Mutex::new(None)),
            inspector_frame: Arc::new(Mutex::new(None)),
            show_occupancy: true,
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
            latched: BTreeSet::new(),
            explore_budget: 1,
            fpv,
            fleet_mode,
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
        self.marks = picture.marks;
        self.presence = picture.presence;
        if self
            .selected
            .is_some_and(|id| !self.marks.iter().any(|mark| mark.id == id))
        {
            self.stop_drive();
            self.selected = None;
            self.drag = None;
        }
        // Only the selected rover's grid is drawn, so only it is subscribed.
        self.feed.focus(self.selected);
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
        let seen: BTreeSet<u64> = self.marks.iter().map(|mark| mark.id).collect();
        self.latched.retain(|id| seen.contains(id));
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
        self.seed_preview();
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
    fn proposal_at(&self, active: &ActiveDrag, cx: &App) -> Option<Point<Pixels>> {
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
    fn selected_mark(&self) -> Option<&RoverMark> {
        self.selected
            .and_then(|id| self.marks.iter().find(|mark| mark.id == id))
    }
    fn select(&mut self, id: u64) {
        if !self.marks.iter().any(|mark| mark.id == id) {
            return;
        }
        if self.selected != Some(id) {
            self.stop_drive();
            self.drag = None;
            self.selected = Some(id);
        }
        self.claim.take(self.now);
        self.note = format!("Rover {id} selected · control active in this window");
    }
    fn reset_safety(&mut self, id: u64) {
        self.stop_drive();
        match self.feed.safety_reset(id) {
            Ok(()) => {
                self.latched.remove(&id);
                self.note = format!("Reset stop sent to rover {id}");
            }
            Err(error) => self.note = error,
        }
    }
    fn stop_rover(&mut self, id: u64) {
        self.stop_drive();
        self.drag = None;
        self.latched.insert(id);
        self.note = match self.feed.emergency_stop(id) {
            Ok(()) => format!("Stop sent to rover {id}"),
            Err(error) => error,
        };
    }
    fn begin_drive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            self.note = "Select a rover before driving".into();
            return;
        };
        if self.latched.contains(&id)
            || self
                .selected_mark()
                .is_some_and(|mark| mark.safety == "emergency_stop")
        {
            self.note = "Stop is active · use Reset stop before driving".into();
            return;
        }
        self.drag = None;
        // Selecting manual control does not send motor commands.
        if self
            .selected_mark()
            .is_some_and(|mark| mark.requested_level.as_deref() != Some("teleop"))
        {
            self.feed.publish_level(id, "teleop");
            self.note = "Selecting manual control · waiting for L0 confirmation".into();
            return;
        }
        match self.feed.begin_drive(id) {
            Ok(()) => {
                self.driving = true;
                self.note = "Drive is manual. In-place spin is off.".into();
                self.focus.focus(window, cx);
            }
            Err(error) => self.note = error,
        }
    }
    fn stop_drive(&mut self) {
        if let Some(id) = self.selected {
            for key in ["w", "a", "s", "d"] {
                let _ = self.feed.drive_key(id, key, false);
            }
        }
        self.feed.end_drive();
        self.driving = false;
        self.held.clear();
    }
    fn estop(&mut self) {
        if let Some(id) = self.selected {
            self.stop_rover(id);
        } else {
            self.stop_drive();
        }
    }
    /// Safety stop for every fleet member, located or not. Each rover then
    /// needs its own Reset stop; nothing here re-arms or resumes.
    fn stop_all(&mut self) {
        self.stop_drive();
        self.drag = None;
        let ids = console::fleet_ids(&self.marks, &self.presence);
        let mut failed = Vec::new();
        for id in &ids {
            self.latched.insert(*id);
            if let Err(error) = self.feed.emergency_stop(*id) {
                failed.push(format!("rover {id} ({error})"));
            }
        }
        self.note = if ids.is_empty() {
            "No rovers in the fleet to stop".into()
        } else if failed.is_empty() {
            format!("Stop sent to all {} rovers", ids.len())
        } else {
            format!("Stop not sent to {}", failed.join(", "))
        };
    }
    /// Reconnect this window to the simulator or the real fleet. Drive input
    /// is zeroed and control released first. Rovers keep whatever level and
    /// goal they had; this only changes which bus the window talks to.
    fn switch_fleet(&mut self, mode: LaunchMode, window: &mut Window) {
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
        self.stop_drive();
        self.drag = None;
        self.selected = None;
        self.claim.release();
        self.latched.clear();
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
    fn set_map_mode(&mut self, mode: MapMode) {
        self.map_mode = mode;
        self.map.set_mode(mode);
    }
    fn is_stopped(&self, mark: &RoverMark) -> bool {
        self.latched.contains(&mark.id) || mark.safety == "emergency_stop"
    }
    fn start_explore(&mut self, id: u64) {
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id) else {
            self.note = "Select a rover that is in the fleet".into();
            return;
        };
        if let Some(reason) = explore_refusal(self.is_stopped(mark), mark.outdoor_gps) {
            self.note = reason.into();
            return;
        }
        let minutes = EXPLORE_BUDGETS_MIN
            .get(self.explore_budget)
            .copied()
            .unwrap_or(EXPLORE_BUDGETS_MIN[0]);
        self.stop_drive();
        self.feed.publish_explore(id, f64::from(minutes) * 60.);
        self.note = format!("Sending L4 Explore · {minutes} min");
    }
    fn hold(&mut self, key: &str, down: bool) {
        let Some(id) = self.selected else {
            return;
        };
        if !self.driving {
            if down {
                self.note = "Click Drive this rover, then hold W/A/S/D in this window".into();
            }
            return;
        }
        match self.feed.drive_key(id, key, down) {
            Ok(()) => {
                if down {
                    self.held.insert(key.to_string());
                    self.feed.refresh_drive(id);
                } else {
                    self.held.remove(key);
                }
                // Clearing the note would show the last background command
                // instead, such as an old "Cancel sent", while driving.
                self.note = if self.held.is_empty() {
                    format!("Driving rover {id} · hold W/A/S/D")
                } else {
                    let keys: Vec<_> = self.held.iter().map(|key| key.to_uppercase()).collect();
                    format!("Driving rover {id} · {}", keys.join(" "))
                };
            }
            Err(error) => {
                self.held.remove(key);
                self.note = error;
            }
        }
    }
}
impl Render for Dashboard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.driving && !window.is_window_active() {
            self.stop_drive();
            self.note = "Drive stopped because the window lost focus".into();
        }
        if self.drag.is_some() && !window.is_window_active() {
            self.cancel_drag();
        }
        let selected = self.selected;
        let control_held = self.claim.held(self.now);
        let driving = self.driving;
        let limits = self.feed.limits();
        let mark = self.selected_mark().cloned();
        let drag_local = self.drag.as_ref().and_then(|active| {
            let goal = &active.goal;
            let local = match goal.space {
                DragSpace::Local => Some((goal.cursor.x, goal.cursor.y)),
                DragSpace::Geographic => self
                    .selected_mark()
                    .and_then(|mark| mark.overlay_frame)
                    .and_then(|frame| frame.local(goal.cursor.x, goal.cursor.y).ok()),
            }?;
            Some((
                goal.id,
                local.0,
                local.1,
                goal.distance_m(),
                goal.arrival_radius_m,
            ))
        });
        let status = console::status_line(&self.note, &self.command).to_owned();
        let unlocated = console::unlocated(&self.marks, &self.presence);
        let notices = console::field_notices(&self.link, &self.detail, self.maps_notice.as_deref());
        let reversing_key = self.driving && self.held.contains("s");
        let handles: Vec<(u64, DragAnchor, Point<Pixels>)> = {
            let map = self.view.read(cx);
            let mut handles = Vec::new();
            for mark in &self.marks {
                if let Some(at) = map.project_local(mark.latitude, mark.longitude) {
                    handles.push((mark.id, DragAnchor::Rover, at));
                }
                if let Some(goal) = goal_geo(mark)
                    && let Some(at) = map.project_local(goal.x, goal.y)
                {
                    handles.push((mark.id, DragAnchor::Goal, at));
                }
            }
            handles
        };
        // robot.glb bounds after the loader's Y-up to Z-up conversion.
        let model_handles: Vec<_> = self
            .marks
            .iter()
            .filter_map(|mark| {
                let object = *self.objects.get(&mark.id)?;
                self.view
                    .read(cx)
                    .object_bounds_local(object, [-1.225, -1.25, 0.], [1.225, 1.25, 2.25])
                    .map(|bounds| (mark.id, bounds))
            })
            .collect();
        let overlay = map_paint(self);
        let confirm = self.drag.as_ref().and_then(|active| {
            let at = self.proposal_at(active, cx)?;
            Some(goal_popup(&active.goal, at, cx).into_any_element())
        });
        let field: Vec<(u64, Option<Point<Pixels>>, console::Callout)> = {
            let map = self.view.read(cx);
            self.marks
                .iter()
                .map(|mark| {
                    let driving_here = driving && selected == Some(mark.id);
                    (
                        mark.id,
                        map.project_local(mark.latitude, mark.longitude),
                        console::callout(mark, self.is_stopped(mark), driving_here),
                    )
                })
                .collect()
        };
        // Measured last frame; until then the field has no known edge.
        let rem = f32::from(window.rem_size());
        if self.selected_mark().is_none() {
            *self.inspector_frame.lock().unwrap() = None;
        }
        let field_bounds = *self.field_frame.lock().unwrap();
        let gap = 0.5 * rem;
        let groups = console::place_cards(
            &field
                .iter()
                .filter_map(|(id, at, _)| at.map(|at| (*id, f32::from(at.x), f32::from(at.y))))
                .collect::<Vec<_>>(),
            console::CardSpace {
                // The card's 1rem offset from its anchor counts toward its size.
                card_width: (CARD_WIDTH_REM + 1.) * rem,
                card_height: CARD_HEIGHT_REM * rem,
                // Both frames are measured at the previous prepaint; until the
                // first one, the field has no known edge and nothing to avoid.
                field: console::FieldRect {
                    x: 0.,
                    y: 0.,
                    width: field_bounds.map_or(f32::INFINITY, |b| f32::from(b.size.width)),
                    height: field_bounds.map_or(f32::INFINITY, |b| f32::from(b.size.height)),
                },
                avoid: field_bounds.zip(*self.inspector_frame.lock().unwrap()).map(
                    |(field, inspector)| console::FieldRect {
                        x: f32::from(inspector.origin.x - field.origin.x) - gap,
                        y: f32::from(inspector.origin.y - field.origin.y) - gap,
                        width: f32::from(inspector.size.width) + 2. * gap,
                        height: f32::from(inspector.size.height) + 2. * gap,
                    },
                ),
            },
        );
        let inspector = mark.map(|mark| {
            rover_inspector(
                self,
                &mark,
                Facts {
                    drag: drag_local,
                    limits,
                    reversing_key,
                },
                cx,
            )
            .into_any_element()
        });
        div()
            .id("argos")
            .track_focus(&self.focus)
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                if let Some(key) = drive_key(&event.keystroke.key) {
                    this.hold(key, false);
                    cx.stop_propagation();
                }
            }))
            // Capture phase: Space stops even when a focused button would
            // otherwise take it as activation.
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "space" {
                    this.estop();
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.cancel_drag();
                    cx.stop_propagation();
                    return;
                }
                if let Some(key) = drive_key(&event.keystroke.key) {
                    this.hold(key, true);
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(".SystemUIFont")
            .child(console_bar(self, status, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    // Bubbles up from the map; cards, tabs, the inspector,
                    // and the popup stop it first.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.press = Some(event.position);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, _, cx| {
                            let Some(press) = this.press.take() else {
                                return;
                            };
                            let moved = f32::from(event.position.x - press.x)
                                .hypot(f32::from(event.position.y - press.y));
                            if moved <= CLICK_SLOP_PX {
                                this.click_map(event.position, cx);
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.press = None),
                    )
                    .child(self.view.clone())
                    .child(overlay)
                    .children(model_handles.into_iter().map(|(id, bounds)| {
                        div()
                            .id(("model-drag", id))
                            .absolute()
                            .left(bounds.origin.x)
                            .top(bounds.origin.y)
                            .w(bounds.size.width)
                            .h(bounds.size.height)
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.focus.focus(window, cx);
                                    this.select(id);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                    }))
                    .children(handles.into_iter().map(|(id, anchor, at)| {
                        let slot = match anchor {
                            DragAnchor::Rover => 2,
                            DragAnchor::Goal => 3,
                        };
                        div()
                            .id(("map-drag", id.saturating_mul(4).saturating_add(slot)))
                            .absolute()
                            .left(at.x - px(9.))
                            .top(at.y - px(9.))
                            .w(px(18.))
                            .h(px(18.))
                            .rounded_md()
                            .cursor_pointer()
                            .bg(
                                if anchor == DragAnchor::Goal
                                    || (selected == Some(id) && control_held)
                                {
                                    cx.theme().success
                                } else {
                                    cx.theme().warning
                                },
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.focus.focus(window, cx);
                                    this.select(id);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                    }))
                    .child({
                        let frame = self.field_frame.clone();
                        canvas(
                            move |bounds, _, _| *frame.lock().unwrap() = Some(bounds),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                    })
                    .children(
                        groups.into_iter().map(|group| {
                            card_group(group, &field, selected, cx).into_any_element()
                        }),
                    )
                    .when(!notices.is_empty(), |field| {
                        field.child(
                            v_flex()
                                .absolute()
                                .top_3()
                                .left_3()
                                .max_w(px(420.))
                                .px_2()
                                .py_1()
                                .gap_1()
                                .popover_style(cx)
                                .text_xs()
                                .text_color(cx.theme().warning)
                                .children(notices),
                        )
                    })
                    .child(offscreen_tabs(&field, &unlocated, selected, cx))
                    .children(confirm)
                    .children(inspector),
            )
    }
}

fn fpv_stats(picture: &argos_gpui::FpvView) -> String {
    let size = if picture.width == 0 {
        "—".to_string()
    } else {
        format!("{}×{}", picture.width, picture.height)
    };
    let codec = if picture.codec.is_empty() {
        "—"
    } else {
        picture.codec.as_str()
    };
    let rtt = picture
        .rtt_ms
        .map(|rtt| format!("{rtt:.0} ms"))
        .unwrap_or_else(|| "—".into());
    format!(
        "{codec} · {size} · {:.1} fps · {:.0} kbps · rtt {rtt}",
        picture.fps, picture.bitrate_kbps
    )
}

fn phase_color(phase: FpvPhase, cx: &App) -> Hsla {
    match phase {
        FpvPhase::Connected => cx.theme().success,
        FpvPhase::Failed => cx.theme().danger,
        FpvPhase::Connecting | FpvPhase::Starting => cx.theme().warning,
        FpvPhase::Idle | FpvPhase::Closed => cx.theme().muted_foreground,
    }
}

/// Window chrome for the console: map camera and grid, the latest status,
/// this window's control claim, link health, and the fleet stop.
fn console_bar(
    dashboard: &Dashboard,
    status: String,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let mode = dashboard.map_mode;
    let fleet = dashboard.fleet_mode;
    let held = dashboard.claim.held(dashboard.now);
    let detail: SharedString = format!("{} · {}", dashboard.link, dashboard.detail).into();
    let status: SharedString = status.into();
    // Video keeps running when its rover leaves the field; without a
    // selection there is no inspector to stop it from.
    let orphan_video = dashboard
        .fpv
        .view()
        .phone_id
        .filter(|_| dashboard.selected.is_none());
    TitleBar::new()
        .child(
            h_flex()
                .gap_3()
                .flex_1()
                .min_w_0()
                .child(div().text_sm().font_semibold().child("ARGOS"))
                .child(
                    ButtonGroup::new("fleet-mode")
                        .outline()
                        .small()
                        .child(
                            Button::new("fleet-sim")
                                .label("Sim")
                                .selected(fleet == LaunchMode::Sim)
                                .tooltip("Local Zorvane simulator at tcp/127.0.0.1:7447"),
                        )
                        .child(
                            Button::new("fleet-real")
                                .label("Real")
                                .selected(fleet == LaunchMode::Real)
                                .tooltip("Real fleet from the environment and .env"),
                        )
                        .on_click(cx.listener(|this, clicks: &Vec<usize>, window, cx| {
                            this.switch_fleet(
                                if clicks.contains(&0) {
                                    LaunchMode::Sim
                                } else {
                                    LaunchMode::Real
                                },
                                window,
                            );
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    ButtonGroup::new("map-mode")
                        .outline()
                        .small()
                        .child(
                            Button::new("map-2d")
                                .label("2D")
                                .selected(mode == MapMode::TwoD),
                        )
                        .child(
                            Button::new("map-3d")
                                .label("3D")
                                .selected(mode == MapMode::ThreeD),
                        )
                        .on_click(cx.listener(|this, clicks: &Vec<usize>, window, cx| {
                            this.set_map_mode(if clicks.contains(&0) {
                                MapMode::TwoD
                            } else {
                                MapMode::ThreeD
                            });
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("occupancy-overlay")
                        .outline()
                        .small()
                        .selected(dashboard.show_occupancy)
                        .label("Map grid")
                        .tooltip("Show the selected rover's occupancy grid on the map")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_occupancy = !this.show_occupancy;
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("console-status")
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .when(!status.is_empty(), |line| {
                            let full = status.clone();
                            line.tooltip(move |window, cx| {
                                Tooltip::new(full.clone()).build(window, cx)
                            })
                        })
                        .child(status),
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .pr_2()
                .flex_none()
                .when_some(orphan_video, |bar, active| {
                    bar.child(
                        stop_fpv_button(cx)
                            .ghost()
                            .tooltip(format!("Video is rover {active}")),
                    )
                })
                .child(
                    Button::new("control-claim")
                        .outline()
                        .small()
                        .selected(held)
                        .label(if held {
                            "Release control"
                        } else {
                            "Take control"
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.claim.held(this.now) {
                                this.claim.release();
                                // Releasing control also closes the rover panel.
                                this.stop_drive();
                                this.drag = None;
                                this.selected = None;
                                this.note = "Control released".into();
                            } else {
                                this.claim.take(this.now);
                                this.note = "Control held by this window".into();
                            }
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("link-health")
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .tooltip(move |window, cx| Tooltip::new(detail.clone()).build(window, cx))
                        .child(format!("Link {}", dashboard.link)),
                )
                .child(
                    Button::new("stop-all")
                        .danger()
                        .small()
                        .label("Stop all")
                        .tooltip("Safety stop every rover. Each needs Reset stop to move again.")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.stop_all();
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                ),
        )
}

fn callout_tag(tag: CalloutTag) -> Tag {
    match tag {
        CalloutTag::Stopped => Tag::danger(),
        CalloutTag::Driving => Tag::info(),
        CalloutTag::Attention => Tag::warning(),
        CalloutTag::Goal => Tag::secondary(),
    }
    .small()
    .child(tag.label())
}

/// Heading uncertainty and age as the rover reports them, against the limits
/// outdoor goals need.
fn heading_line(report: &OutdoorReport) -> String {
    let uncertainty = report.heading_accuracy_deg.map_or_else(
        || "uncertainty not reported".into(),
        |degrees| format!("±{degrees:.0}°"),
    );
    let age = report.heading_age_s.map_or_else(
        || "age not reported".into(),
        |age| format!("{age:.1} s old"),
    );
    format!(
        "{uncertainty} · {age} · limit ±{HEADING_MAX_UNCERTAINTY_DEG:.0}°, {HEADING_MAX_AGE_S:.0} s"
    )
}

/// How far the pointer may move between press and release and still count
/// as a click rather than a camera orbit or pan.
const CLICK_SLOP_PX: f32 = 4.;

/// The proposed destination's popup, anchored just above the clicked point.
fn goal_popup(goal: &GoalDrag, at: Point<Pixels>, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let distance = match goal.arrival_radius_m {
        Some(radius) => format!(
            "{:.1} m GPS · arrival radius {radius:.0} m",
            goal.distance_m()
        ),
        None => format!("{:.1} m from rover {}", goal.distance_m(), goal.id),
    };
    div()
        .absolute()
        .left(at.x)
        .top(at.y)
        .size_0()
        .child(
            v_flex()
                .id("goal-confirm")
                .absolute()
                .bottom_3()
                .left(px(-96.))
                .w(px(192.))
                .p_2()
                .gap_2()
                .popover_style(cx)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    v_flex()
                        .child(div().text_sm().child("Send goal here?"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(distance),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("goal-cancel")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_drag();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("goal-send")
                                .primary()
                                .small()
                                .label("Send goal")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.release_drag();
                                    cx.notify();
                                })),
                        ),
                ),
        )
        .child(
            div()
                .absolute()
                .left(px(-4.))
                .top(px(-4.))
                .size_2()
                .rounded_full()
                .bg(cx.theme().success),
        )
}

/// Widest a rover card draws, so placement can keep it clear of the
/// inspector. Longer states truncate.
const CARD_WIDTH_REM: f32 = 14.;
/// One card's height plus the gap between stacked cards.
const CARD_HEIGHT_REM: f32 = 3.5;

/// Rover cards that share one projected anchor, stacked beside it on the side
/// `place_cards` chose to stay inside the field and clear of the inspector. Choosing a card selects
/// that rover, as clicking the rover does.
fn card_group(
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
        .selected(selected)
        .child(
            v_flex()
                .items_start()
                .gap_1()
                .min_w_0()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(format!("Rover {id}")),
                        )
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
fn offscreen_tabs(
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
                        .selected(selected == Some(id))
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

/// Live telemetry the inspector draws but does not own.
struct Facts {
    drag: Option<(u64, f64, f64, f64, Option<f64>)>,
    limits: argos_core::TeleopLimits,
    reversing_key: bool,
}

/// The selected rover: identity, state, and the commands that act on it on
/// every tab; the rest of its controls are grouped by tab and section.
fn rover_inspector(
    dashboard: &Dashboard,
    mark: &RoverMark,
    facts: Facts,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
    let stopped = dashboard.is_stopped(mark);
    let driving = dashboard.driving;
    let tab = dashboard.inspector_tab;
    let callout = console::callout(mark, stopped, driving);
    let manual = mark.requested_level.as_deref() == Some("teleop");
    let primary = if stopped {
        Button::new(("inspector-reset", id))
            .label("Reset stop")
            .on_click(cx.listener(move |this, _, window, cx| {
                this.reset_safety(id);
                this.focus.focus(window, cx);
                cx.notify();
            }))
    } else if driving {
        Button::new(("inspector-leave", id))
            .label("Leave drive")
            .on_click(cx.listener(|this, _, window, cx| {
                this.stop_drive();
                this.note = "Drive off. Zero twist sent.".into();
                this.focus.focus(window, cx);
                cx.notify();
            }))
    } else {
        Button::new(("inspector-drive", id))
            .label(if manual { "Drive" } else { "Take over" })
            .tooltip(if manual {
                "Hold W/A/S/D in this window to drive. Release stops."
            } else {
                "Requests L0 manual control. Drive is available once the rover confirms."
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.begin_drive(window, cx);
                cx.notify();
            }))
    };
    let reversing = mark.reversing || (facts.reversing_key && driving);
    let body = match tab {
        InspectorTab::Overview => overview_tab(dashboard, mark, &facts, cx).into_any_element(),
        InspectorTab::Video => video_tab(dashboard, cx).into_any_element(),
    };
    v_flex()
        .absolute()
        .top_3()
        .right_3()
        .bottom_3()
        .w(px(360.))
        .popover_style(cx)
        .overflow_hidden()
        // Measured so rover cards can stay out from under the inspector.
        .child({
            let frame = dashboard.inspector_frame.clone();
            canvas(
                move |bounds, _, _| *frame.lock().unwrap() = Some(bounds),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            v_flex()
                .flex_none()
                .p_4()
                .pb_2()
                .gap_3()
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(format!("Rover {id}")),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .when(reversing, |tags| {
                                    tags.child(Tag::warning().small().child("Reversing"))
                                })
                                .children(callout.tag.map(callout_tag)),
                        ),
                )
                .child(div().text_base().font_medium().child(callout.state))
                .child(
                    h_flex().gap_2().child(primary.outline().small()).child(
                        Button::new(("inspector-stop", id))
                            .danger()
                            .small()
                            .label("Stop")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.stop_rover(id);
                                this.focus.focus(window, cx);
                                cx.notify();
                            })),
                    ),
                ),
        )
        .child(
            TabBar::new("inspector-tabs")
                .underline()
                .small()
                .px_2()
                .selected_index(tab.index())
                .children(
                    InspectorTab::ALL
                        .into_iter()
                        .map(|tab| Tab::new().label(tab.label())),
                )
                .on_click(cx.listener(|this, index: &usize, _, cx| {
                    this.inspector_tab = InspectorTab::from_index(*index);
                    cx.notify();
                })),
        )
        .child(
            v_flex()
                .id("inspector-body")
                // Zero basis: the body takes only the height left under the
                // header and tab bar, and scrolls the rest.
                .h_0()
                .flex_1()
                .min_h_0()
                .p_4()
                .gap_5()
                .child(body)
                .overflow_y_scrollbar(),
        )
}

fn section(title: &'static str, cx: &App) -> Div {
    v_flex().gap_2().child(
        div()
            .text_xs()
            .font_semibold()
            .text_color(cx.theme().muted_foreground)
            .child(title),
    )
}

fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

fn caution(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().warning)
        .child(text.into())
}

fn rows(items: Vec<(&'static str, String)>) -> DescriptionList {
    DescriptionList::horizontal()
        .columns(1)
        .bordered(false)
        .label_width(rems(6.))
        .children(
            items
                .into_iter()
                .map(|(label, value)| DescriptionItem::new(label).value(value)),
        )
}

/// What a goal drag would do now, or why it cannot start.
fn goal_line(
    mark: &RoverMark,
    drag: Option<(u64, f64, f64, f64, Option<f64>)>,
    link: &str,
) -> String {
    let level = mark
        .effective_level
        .clone()
        .or_else(|| mark.requested_level.clone());
    let local_gate = DragGate {
        selected: true,
        link_healthy: link == "healthy",
        pose_fresh: mark.status == "online",
        degraded: mark.degraded,
        level: level.as_deref(),
        geographic: false,
        outdoor: false,
        admission: None,
    };
    let drive_reason = drive_block(&mark.hardware);
    if let Some((_, _, _, distance, radius)) = drag.filter(|item| item.0 == mark.id) {
        match radius {
            Some(radius) => format!("{distance:.1} m GPS · arrival radius {radius:.0} m"),
            None => format!("{distance:.1} m"),
        }
    } else if mark.outdoor_gps {
        match drag_disabled_reason(geographic_gate(mark, link, true)) {
            Some(reason) => reason.to_string(),
            None => drive_reason
                .unwrap_or("Drag the map marker. Release sends the GPS goal.")
                .into(),
        }
    } else {
        match drag_disabled_reason(local_gate) {
            Some(reason) => reason.to_string(),
            None => drive_reason
                .unwrap_or("Drag the rover or its goal. Release sends.")
                .into(),
        }
    }
}

fn overview_tab(
    dashboard: &Dashboard,
    mark: &RoverMark,
    facts: &Facts,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
    let stopped = dashboard.is_stopped(mark);
    let driving = dashboard.driving;
    let (hardware_status, hardware_detail) = console::hardware_lines(mark, stopped);
    let level = console::level_index(mark);
    let source = console::source_index(mark);
    let mut navigation = Vec::new();
    if let Some(gps) = mark.raw_gps {
        navigation.push((
            "Position",
            format!(
                "{} · {:.5}, {:.5}{}{}",
                position_label(mark),
                gps.latitude,
                gps.longitude,
                gps.accuracy_m
                    .map(|accuracy| format!(" · ±{accuracy:.1} m"))
                    .unwrap_or_default(),
                gps.age_s
                    .map(|age| format!(" · {age:.1} s"))
                    .unwrap_or_default()
            ),
        ));
    } else {
        navigation.push(("Position", position_label(mark)));
    }
    if let Some(report) = mark.outdoor.as_ref() {
        navigation.push(("Heading", heading_line(report)));
        navigation.push(("Phase", phase_label(&report.phase)));
        if let Some(metres) = report.remaining_m {
            navigation.push(("Remaining", format!("{metres:.0} m")));
        }
        if let (Some(latitude), Some(longitude)) =
            (report.destination_latitude, report.destination_longitude)
        {
            navigation.push(("Destination", format!("{latitude:.5}, {longitude:.5}")));
        }
        navigation.push((
            "Arrival radius",
            format!("{:.0} m", report.arrival_radius_m),
        ));
    }
    v_flex()
        .gap_5()
        .child(rows(console::trace(mark, stopped)))
        .child(
            section("Hardware", cx)
                .child(div().text_sm().child(hardware_status.to_owned()))
                .when_some(hardware_detail, |section, detail| {
                    section.child(caution(detail.to_owned(), cx))
                })
                // Reset stop clears this window's latch or the rover's own stop.
                .when(stopped, |section| {
                    section.child(
                        h_flex().child(
                            Button::new(("reset-stop", id))
                                .outline()
                                .small()
                                .label("Reset stop")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.reset_safety(id);
                                    cx.notify();
                                })),
                        ),
                    )
                }),
        )
        .child(
            section("Autonomy", cx)
                .child(hint(console::level_line(mark), cx))
                // L0–L3, then the supervised levels, so neither row wraps.
                .child(level_group(id, level, 0..4, cx))
                .child(level_group(id, level, 4..OPERATOR_LEVELS.len(), cx)),
        )
        .child(explore_section(dashboard, mark, stopped, cx))
        .child(
            section("Navigation", cx)
                .child(
                    ButtonGroup::new(("source", id))
                        .outline()
                        .small()
                        .children(console::NAVIGATION_SOURCES.iter().enumerate().map(
                            |(index, (_, label))| {
                                Button::new(("source", index as u64))
                                    .label(*label)
                                    .selected(source == index)
                            },
                        ))
                        .on_click(cx.listener(move |this, clicks: &Vec<usize>, _, cx| {
                            if this.selected == Some(id)
                                && let Some((wire, _)) = clicks
                                    .first()
                                    .and_then(|index| console::NAVIGATION_SOURCES.get(*index))
                            {
                                this.feed.select_source(id, wire);
                            }
                            cx.notify();
                        })),
                )
                .child(hint(console::navigation_note(mark), cx))
                .when(mark.app_update_required && mark.outdoor_gps, |section| {
                    section.child(caution(APP_UPDATE_REQUIRED, cx))
                })
                .when_some(
                    mark.outdoor.as_ref().and_then(|report| {
                        let label = report.operator_hold_label()?;
                        Some(if label == "Heading uncertain" {
                            format!("{label} · {}", heading_line(report))
                        } else {
                            label.to_owned()
                        })
                    }),
                    |section, label| section.child(caution(label, cx)),
                )
                .child(rows(navigation))
                .when(
                    mark.outdoor
                        .as_ref()
                        .is_some_and(|report| report.phase == "paused"),
                    |section| {
                        section.child(
                            h_flex().child(
                                Button::new("resume-outdoor")
                                    .outline()
                                    .small()
                                    .label("Resume outdoor mission")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.feed.resume_outdoor(id);
                                        cx.notify();
                                    })),
                            ),
                        )
                    },
                ),
        )
        .child(goal_section(dashboard, mark, facts, cx))
        .when(driving, |tab| tab.child(drive_section(dashboard, mark, facts.limits, cx)))
        .child(hint(
            "Click the map to place a destination, then Send goal in the popup. The rover stays at its live position. Outdoor GPS sends WGS84. Esc or Cancel sends nothing. Drive sends commands while keys are held. Stop halts the rover; Reset stop clears the stop.",
            cx,
        ))
}

/// One row of the autonomy picker over `OPERATOR_LEVELS[range]`; the
/// requested level is selected.
fn level_group(
    id: u64,
    requested: Option<usize>,
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
            .map(|(offset, (_, label))| {
                let index = start + offset;
                Button::new(("level", index as u64))
                    .label(*label)
                    .selected(requested == Some(index))
            }),
    )
    .on_click(cx.listener(move |this, clicks: &Vec<usize>, _, cx| {
        if this.selected == Some(id)
            && let Some((wire, _)) = clicks
                .first()
                .and_then(|offset| OPERATOR_LEVELS.get(start + offset))
        {
            if *wire == "explore" {
                this.start_explore(id);
            } else {
                this.feed.publish_level(id, wire);
                this.note = format!("Sending {}", level_label(wire));
            }
        }
        cx.notify();
    }))
}

/// L4 budget picker and Terra's run status. Terra chooses the frontiers; the
/// target also shows as the rover's goal on the map.
fn explore_section(
    dashboard: &Dashboard,
    mark: &RoverMark,
    stopped: bool,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let refusal = explore_refusal(stopped, mark.outdoor_gps);
    let exploring = mark.requested_level.as_deref() == Some("explore");
    section("Exploration · L4", cx)
        .child(
            ButtonGroup::new("explore-budget")
                .outline()
                .small()
                .children(EXPLORE_BUDGETS_MIN.iter().enumerate().map(|(index, minutes)| {
                    Button::new(("explore-budget", index as u64))
                        .label(format!("{minutes} min"))
                        .selected(dashboard.explore_budget == index)
                }))
                .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                    if let Some(index) = clicks
                        .first()
                        .filter(|index| **index < EXPLORE_BUDGETS_MIN.len())
                    {
                        this.explore_budget = *index;
                    }
                    cx.notify();
                })),
        )
        .when_some(refusal.filter(|_| !exploring), |section, reason| {
            section.child(caution(reason, cx))
        })
        .map(|section| match mark.exploration.as_ref() {
            Some(report) => {
                let stuck = report.running()
                    && matches!(
                        report.reason.as_deref(),
                        Some("frontier_retry" | "obstacle_blocked")
                    );
                section
                    .when(stuck, |section| {
                        section.child(caution(report.state_label(), cx))
                    })
                    .child(rows(report.rows()))
            }
            None => section.child(hint(
                "L4 Explore drives to unmapped frontiers on its own until the budget ends or no frontier is left. Stop, Cancel goal, or choosing another level ends the run.",
                cx,
            )),
        })
}

fn goal_section(
    dashboard: &Dashboard,
    mark: &RoverMark,
    facts: &Facts,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
    section("Goal", cx)
        .child(
            h_flex()
                .gap_2()
                .justify_between()
                .child(div().min_w_0().flex_1().text_xs().child(goal_line(
                    mark,
                    facts.drag,
                    &dashboard.link,
                )))
                .child(
                    Button::new("cancel-goal")
                        .outline()
                        .small()
                        .label("Cancel goal")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.feed.cancel_goal(id);
                            this.drag = None;
                            this.note = "Cancel requested".into();
                            cx.notify();
                        })),
                ),
        )
        .when(console::path_provisional(mark), |section| {
            section.child(hint(
                "Path direction is provisional until a segment velocity is published.",
                cx,
            ))
        })
}

/// Manual drive settings and the on-screen keys. Shown only while this
/// window drives the rover.
fn drive_section(
    dashboard: &Dashboard,
    mark: &RoverMark,
    limits: argos_core::TeleopLimits,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let speed = console::speed_index(limits.linear);
    section("Drive · manual", cx)
        .child(
            ButtonGroup::new("drive-speed")
                .outline()
                .small()
                .children(
                    console::DRIVE_SPEEDS
                        .iter()
                        .enumerate()
                        .map(|(index, value)| {
                            Button::new(("speed", index as u64))
                                .label(format!("{value:.1} m/s"))
                                .selected(speed == Some(index))
                        }),
                )
                .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                    if let Some(value) = clicks
                        .first()
                        .and_then(|index| console::DRIVE_SPEEDS.get(*index))
                        && let Err(error) = this.feed.set_linear(*value)
                    {
                        this.note = error;
                    }
                    cx.notify();
                })),
        )
        .when(
            mark.requested_level.as_deref() == Some("teleop") && mark.status == "pose stale",
            |section| {
                section.child(hint(
                    "Camera pose unavailable · manual control does not need tracking",
                    cx,
                ))
            },
        )
        .child(turning_row(limits, cx))
        .child(autonomy_turning_row(mark, cx))
        .child(hint(
            "Hold W/A/S/D to drive. Release to stop. Reverse uses the same turning radius.",
            cx,
        ))
        .child(drive_pad(&dashboard.held, cx))
}

fn turning_row(limits: argos_core::TeleopLimits, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let yaw = cycle(limits.yaw_rate, &[0.25, 0.5, 1.0]);
    let accel = cycle(limits.yaw_accel, &[0.25, 0.5, 1.0]);
    let decel = cycle(limits.yaw_decel, &[0.4, 0.8, 1.6]);
    let radius = cycle(limits.min_turn_radius, &[0.35, 0.5, 1.0]);
    h_flex()
        .flex_wrap()
        .gap_1()
        .child(limit_chip(
            "yaw-rate",
            format!("Yaw {:.2}", limits.yaw_rate),
            cx,
            move |this| this.feed.set_yaw_rate(yaw),
        ))
        .child(limit_chip(
            "yaw-accel",
            format!("Accel {:.2}", limits.yaw_accel),
            cx,
            move |this| this.feed.set_yaw_accel(accel),
        ))
        .child(limit_chip(
            "yaw-decel",
            format!("Decel {:.2}", limits.yaw_decel),
            cx,
            move |this| this.feed.set_yaw_decel(decel),
        ))
        .child(limit_chip(
            "turn-radius",
            format!("Radius {:.2}", limits.min_turn_radius),
            cx,
            move |this| this.feed.set_min_radius(radius),
        ))
        .child(
            Button::new("spin")
                .outline()
                .small()
                .selected(limits.allow_spin)
                .label(if limits.allow_spin {
                    "In-place spin on"
                } else {
                    "In-place spin off"
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    let allow = !this.feed.limits().allow_spin;
                    if let Err(error) = this.feed.set_spin(allow) {
                        this.note = error;
                    }
                    cx.notify();
                })),
        )
}

/// Terra's planner turning, which governs autonomous goals. Teleop spin above
/// only shapes the operator's own commands. Sending also sets CrabTrapBot's size.
fn autonomy_turning_row(mark: &RoverMark, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let id = mark.id;
    let reported = argos_core::navigation_allows_in_place(&mark.navigation_settings);
    let footprint = mark.navigation_settings["footprint_radius"].as_f64();
    h_flex()
        .flex_wrap()
        .items_center()
        .gap_1()
        .child(
            Button::new("autonomy-spin")
                .outline()
                .small()
                .selected(reported == Some(true))
                .label(match reported {
                    Some(true) => "Autonomy spin on",
                    Some(false) => "Autonomy spin off",
                    None => "Autonomy spin unknown",
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.feed.configure_navigation(id, reported != Some(true));
                    cx.notify();
                })),
        )
        .child(hint(
            format!(
                "{}sets CrabTrapBot {:.2} × {:.2} m",
                footprint.map_or(String::new(), |r| format!("Footprint {r:.2} m · ")),
                argos_core::CRAB_TRAP_BOT_LENGTH,
                argos_core::CRAB_TRAP_BOT_WIDTH,
            ),
            cx,
        ))
}

/// A drive limit that steps to its next preset on each click.
fn limit_chip(
    id: &'static str,
    label: String,
    cx: &mut Context<Dashboard>,
    apply: impl Fn(&mut Dashboard) -> Result<(), String> + 'static,
) -> impl IntoElement {
    Button::new(id)
        .outline()
        .small()
        .compact()
        .label(label)
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Err(error) = apply(this) {
                this.note = error;
            }
            cx.notify();
        }))
}

fn cycle(current: f64, choices: &[f64]) -> f64 {
    choices
        .iter()
        .copied()
        .find(|value| *value > current + 0.001)
        .unwrap_or(choices[0])
}

/// On-screen W/A/S/D. Each key drives while the pointer holds it, like the
/// keyboard; a held key shows as selected.
fn drive_pad(held: &BTreeSet<String>, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let key = |name: &'static str, label: &'static str, cx: &mut Context<Dashboard>| {
        Button::new(name)
            .outline()
            .small()
            .w(px(36.))
            .label(label)
            .selected(held.contains(name))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.hold(name, true);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.hold(name, false);
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.hold(name, false);
                    cx.notify();
                }),
            )
    };
    v_flex()
        .gap_1()
        .items_center()
        .child(key("w", "W", cx))
        .child(
            h_flex()
                .gap_1()
                .child(key("a", "A", cx))
                .child(key("s", "S", cx))
                .child(key("d", "D", cx)),
        )
}

/// Video stays with the selection: Start FPV asks the selected rover's phone.
fn video_tab(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let picture = dashboard.fpv.view();
    let phase = picture.phase;
    let detail = picture.detail.clone();
    v_flex()
        .gap_3()
        .child(hint(fpv_subject(dashboard.selected, picture.phone_id), cx))
        .child(
            div()
                .w_full()
                .h(px(188.))
                .bg(cx.theme().background)
                .rounded(cx.theme().radius)
                .overflow_hidden()
                .when_some(picture.image.clone(), |frame, image| {
                    frame.child(
                        img(image)
                            .id("fpv-picture")
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    )
                })
                .when(picture.image.is_none(), |frame| {
                    frame.child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .px_3()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(detail.clone()),
                    )
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(phase_color(phase, cx))
                .child(format!("{} · {detail}", phase.label())),
        )
        .child(hint(fpv_stats(&picture), cx))
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("fpv-start")
                        .outline()
                        .small()
                        .label("Start FPV")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(id) = this.selected {
                                this.fpv.start_phone(id);
                                this.note = format!("FPV requested for rover {id}");
                            } else {
                                this.note = "Select a rover before starting FPV".into();
                            }
                            cx.notify();
                        })),
                )
                .child(stop_fpv_button(cx)),
        )
        .child(hint(
            "Video only. Drive, arm, and stop stay on the Zenoh controls.",
            cx,
        ))
}

fn stop_fpv_button(cx: &mut Context<Dashboard>) -> Button {
    Button::new("fpv-stop")
        .outline()
        .small()
        .label("Stop FPV")
        .on_click(cx.listener(|this, _, _, cx| {
            this.fpv.stop();
            this.note = "FPV stopped".into();
            cx.notify();
        }))
}

fn fpv_subject(selected: Option<u64>, watching: Option<u64>) -> String {
    match (selected, watching) {
        (Some(id), Some(active)) if id != active => {
            format!("Selected rover {id}. Video is rover {active}.")
        }
        (Some(id), _) => format!("Selected rover {id}"),
        (None, Some(active)) => format!("Video is rover {active}"),
        (None, None) => "Select a rover, then start video.".into(),
    }
}

fn drive_key(key: &str) -> Option<&'static str> {
    match key {
        "w" | "up" => Some("w"),
        "s" | "down" => Some("s"),
        "a" | "left" => Some("a"),
        "d" | "right" => Some("d"),
        _ => None,
    }
}

fn main() {
    let mode = match LaunchMode::parse(std::env::args().skip(1)) {
        Ok(Some(mode)) => mode,
        Ok(None) => {
            print!("{}", argos_gpui::LAUNCH_HELP);
            return;
        }
        Err(error) => {
            eprintln!("{error}\n{}", argos_gpui::LAUNCH_HELP);
            std::process::exit(2);
        }
    };
    let config = DashboardConfig::from_env_mode(mode).unwrap_or_else(|error| {
        eprintln!("argos-gpui: {error}");
        std::process::exit(1);
    });
    // Resolve the tiles key before MapConfig reads the process environment.
    let maps_api_key = config.maps_api_key.clone();
    let maps_notice = config.maps_notice.clone();
    let feed = FleetFeed::start(config.clone());
    let fpv = FpvController::start(config.clone());
    let map = Map::new(MapConfig {
        latitude: config.start_latitude,
        longitude: config.start_longitude,
        show_controls: false,
        api_key: maps_api_key,
        ..MapConfig::default()
    })
    .unwrap_or_else(|error| {
        eprintln!("argos-gpui: {error}");
        std::process::exit(1);
    });
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.), px(480.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(mode.title().into()),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
            },
            cx,
            |window, cx| {
                // Kit widgets follow the light theme on the console background.
                // The status colors are darkened so they read as text too.
                Theme::change(ThemeMode::Light, Some(window), cx);
                Theme::update(cx, |theme| {
                    theme.background = rgb(palette::BACKGROUND).into();
                    theme.title_bar = rgb(palette::BACKGROUND).into();
                    theme.foreground = rgb(palette::TEXT).into();
                    theme.muted_foreground = rgb(palette::MUTED).into();
                    let status = |color: u32| {
                        let color: Hsla = rgb(color).into();
                        (color, color.opacity(0.9), color.opacity(0.8))
                    };
                    (theme.warning, theme.warning_hover, theme.warning_active) =
                        status(palette::WARNING);
                    (theme.success, theme.success_hover, theme.success_active) =
                        status(palette::SUCCESS);
                    (theme.danger, theme.danger_hover, theme.danger_active) =
                        status(palette::DANGER);
                    theme.warning_foreground = gpui::white();
                });
                let view = cx.new(|cx| Dashboard::new(map, feed, fpv, maps_notice, mode, cx));
                let focus = view.read(cx).focus.clone();
                focus.focus(window, cx);
                view
            },
        )
        .expect("Failed to open window");
        cx.activate(true);
    });
}
