//! ARGOS operator dashboard.
//!
//! The camera starts at `ARGOS_MAP_START_LAT` / `ARGOS_MAP_START_LON` until rover
//! positions arrive, then recenters on the fleet. `MAPS_DEMO=1` still draws the
//! offline fixture. Clicking a rover selects it and takes local control.
//! Control-dragging its body or marker previews a goal; release sends it.
//! Esc, or dropping the ghost back on the rover, sends nothing. An acknowledged
//! outdoor GPS rover uses the same gesture on the map: the ghost shows the
//! arrival radius and GPS distance, and release publishes the WGS84 point.
//! That release is the explicit operator action. The outdoor GPS spec's
//! separate confirm step is not used.

mod interaction;
mod local_map;
mod map_overlay;

use interaction::{geographic_gate, goal_geo, gps_point};
use local_map::local_grid;
use map_overlay::map_paint;

use argos_core::{
    APP_UPDATE_REQUIRED, ControlClaim, DragAnchor, DragGate, DragSpace, GoalDrag, HEADING_HOLD,
    HEADING_HOLD_LABEL, OPERATOR_LEVELS, PlanarPoint, Travel, drag_disabled_reason, drive_block,
    level_label, phase_label, pose_warning_text,
};
use argos_gpui::console::{self, CalloutTag};
use argos_gpui::{
    DashboardConfig, FleetFeed, FpvController, FpvPhase, RoverMark, RoverPresence, model_position,
    position_label, reframing_target,
};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Selectable, Sizable, StyledExt, Theme, ThemeMode, ThemeStyled, TitleBar,
    button::{Button, ButtonGroup, ButtonVariants},
    description_list::{DescriptionItem, DescriptionList},
    tag::Tag,
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
    grab_offset: Option<Point<Pixels>>,
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
    grid_frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
    /// The map area and the inspector, measured at prepaint for rover card
    /// placement.
    field_frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
    inspector_frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
    show_occupancy: bool,
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
    fpv: FpvController,
    _poll: Task<()>,
}
impl Dashboard {
    fn new(
        map: Map,
        feed: FleetFeed,
        fpv: FpvController,
        maps_notice: Option<String>,
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
            grid_frame: Arc::new(Mutex::new(None)),
            field_frame: Arc::new(Mutex::new(None)),
            inspector_frame: Arc::new(Mutex::new(None)),
            show_occupancy: true,
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
            fpv,
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
            grab_offset: None,
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
                    mark.map_frame,
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
    fn set_map_mode(&mut self, mode: MapMode) {
        self.map_mode = mode;
        self.map.set_mode(mode);
    }
    fn is_stopped(&self, mark: &RoverMark) -> bool {
        self.latched.contains(&mark.id) || mark.safety == "emergency_stop"
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
                self.note.clear();
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
                    .and_then(|mark| mark.map_frame)
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
        let note = if self.note.is_empty() {
            self.command.clone()
        } else {
            self.note.clone()
        };
        let grid_frame = self.grid_frame.clone();
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
        let inspector = self.selected_mark().cloned().map(|mark| {
            let stopped = self.is_stopped(&mark);
            rover_inspector(&mark, stopped, driving, self.inspector_frame.clone(), cx)
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
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.drag.is_some() {
                    this.move_drag(event.position, cx);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.drag.is_some() {
                        this.release_drag();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.drag.is_some() {
                        this.release_drag();
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0b141d))
            .text_color(rgb(0xd8e7ec))
            .font_family(".SystemUIFont")
            .child(console_bar(self, cx))
            .child(
                h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
            .child(
                div()
                    .id("sidebar")
                    .w(px(340.))
                    .h_full()
                    .flex_none()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .overflow_y_scroll()
                    .child(div().text_size(px(22.)).child("ARGOS"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(0x98aeb9))
                            .child(format!("{} · {}", self.link, self.detail)),
                    )
                    .when_some(self.maps_notice.clone(), |this, notice| {
                        this.child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(0xf0c36a))
                                .child(notice),
                        )
                    })
                    .when(!note.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(0xf0c36a))
                                .child(note),
                        )
                    })
                    .child(button(
                        "control-claim",
                        if self.claim.held(self.now) {
                            "Release control"
                        } else {
                            "Take control"
                        },
                        rgb(if self.claim.held(self.now) {
                            0x245c49
                        } else {
                            0x1b2c38
                        }),
                        cx.listener(|this, _, _, cx| {
                            if this.claim.held(this.now) {
                                this.claim.release();
                                this.note = "Control released".into();
                            } else {
                                this.claim.take(this.now);
                                this.note = "Control held by this window".into();
                            }
                            cx.notify();
                        }),
                    ))
                    .children(self.presence.iter().filter(|row| !self.marks.iter().any(|mark| mark.id == row.id())).map(|row| {
                        div().id(("unlocated-rover", row.id())).p_2().border_1()
                            .border_color(cx.theme().border).rounded_md().text_size(px(12.))
                            .text_color(cx.theme().foreground)
                            .child(format!("Rover {} · {}", row.id(), row.membership()))
                            .child(div().text_color(cx.theme().muted_foreground).child(row.position_reason().to_owned()))
                            .child(div().text_color(cx.theme().muted_foreground).child(
                                row.observation_age().map(|age| format!("Last observation {age:.1} s ago")).unwrap_or_else(|| "Waiting for telemetry".into())))
                    }))
                    .children(self.marks.iter().map(|mark| {
                        let id = mark.id;
                        let attention = mark.status != "online";
                        let chosen = selected == Some(id);
                        let stopped = self.latched.contains(&id) || mark.safety == "emergency_stop";
                        let hardware_status = if stopped { "Stop active" } else { mark.hardware.status_label() };
                        let hardware_detail = if stopped { "Use Reset stop before driving" } else { drive_block(&mark.hardware).unwrap_or("") };
                        div()
                            .id(("rover", id))
                            .p_2()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if chosen {
                                0x3dcc9a
                            } else if attention {
                                0xc47b2a
                            } else {
                                0x263d48
                            }))
                            .bg(rgb(0x14232e))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select(id);
                                cx.notify();
                            }))
                            .child(format!(
                                "Rover {id} · {} · {}",
                                self.presence.iter().find(|row| row.id() == id).map(|row| row.membership()).unwrap_or(mark.status.as_str()), mark.occupancy
                            ))
                            .child(div().text_color(cx.theme().muted_foreground).child(self.presence.iter().find(|row| row.id() == mark.id).map(|row| row.position_reason().to_owned()).unwrap_or_else(|| position_label(mark))))
                            .child(div().text_color(rgb(0x98aeb9)).child(level_line(mark)))
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(hardware_status),
                            )
                            .child(hardware_row(id, cx))
                            .when(!hardware_detail.is_empty(), |row| {
                                let detail = hardware_detail.to_owned();
                                row.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(cx.theme().warning)
                                        .child(detail),
                                )
                            })
                            .when(mark.reversing || (reversing_key && chosen), |row| {
                                row.child(div().text_color(rgb(0xf0c36a)).child("Reversing"))
                            })
                            .when(mark.hold.as_deref() == Some(HEADING_HOLD), |row| {
                                row.child(div().text_color(rgb(0xe7b15a)).child(HEADING_HOLD_LABEL))
                            })
                            .when_some(
                                pose_warning_text(mark.command_origin, mark.degraded, &mark.degrade_reason),
                                |row, text| row.child(div().text_color(rgb(0x98aeb9)).child(text)),
                            )
                    }))
                    .when(self.marks.is_empty(), |this| {
                        this.child(
                            div().text_size(px(12.)).text_color(rgb(0x98aeb9)).child(
                                "No geographic position yet. Rover connection status is listed above.",
                            ),
                        )
                    })
                    .children(mark.map(|mark| {
                        rover_panel(
                            mark,
                            drag_local,
                            driving,
                            limits,
                            &self.link,
                            grid_frame.clone(),
                            cx,
                        )
                    }))
                    .child(button(
                        "occupancy-overlay",
                        if self.show_occupancy {
                            "Map grid on"
                        } else {
                            "Map grid off"
                        },
                        rgb(if self.show_occupancy { 0x245c49 } else { 0x1b2c38 }),
                        cx.listener(|this, _, _, cx| {
                            this.show_occupancy = !this.show_occupancy;
                            cx.notify();
                        }),
                    ))
                    .child(
                        div().mt_2().text_size(px(11.)).text_color(rgb(0x98aeb9)).child(
                            "Control-drag a 3D rover to place its destination; release to send. The ground marker stays at its live position. Outdoor GPS sends WGS84. Esc cancels. Drive sends commands while keys are held. Stop halts the rover; Reset stop clears the stop.",
                        ),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.view.clone())
                    .child(overlay)
                    .children(model_handles.into_iter().map(|(id, bounds)| {
                        div()
                            .id(("model-drag", id))
                            .absolute()
                            .left(bounds.origin.x).top(bounds.origin.y)
                            .w(bounds.size.width).h(bounds.size.height)
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                this.focus.focus(window, cx);
                                this.pick_model(id, event, cx);
                                cx.stop_propagation();
                                cx.notify();
                            }))
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
                            .bg(rgb(if anchor == DragAnchor::Goal || (selected == Some(id) && control_held) {
                                0x3dcc9a
                            } else {
                                0xf0b429
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    this.focus.focus(window, cx);
                                    if anchor == DragAnchor::Rover {
                                        this.pick_model(id, event, cx);
                                    } else {
                                        this.select(id);
                                        this.begin_drag(id, anchor, true);
                                    }
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
                    .children(groups.into_iter().map(|group| {
                        card_group(group, &field, selected, cx).into_any_element()
                    }))
                    .child(offscreen_tabs(&field, selected, cx))
                    .children(inspector),
            )
            .child(fpv_panel(self, cx)),
            )
    }
}

fn fpv_panel(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let picture = dashboard.fpv.view();
    let selected = dashboard.selected;
    let phase = picture.phase;
    let detail = picture.detail.clone();
    let stats = fpv_stats(&picture);
    let watching = picture.phone_id;
    div()
        .id("fpv")
        .w(px(360.))
        .h_full()
        .flex_none()
        .p_4()
        .flex()
        .flex_col()
        .gap_2()
        .bg(rgb(0x101c25))
        .border_l_1()
        .border_color(rgb(0x263d48))
        .child(div().text_size(px(16.)).child("FPV"))
        .child(div().text_size(px(12.)).text_color(rgb(0x98aeb9)).child(
            match (selected, watching) {
                (Some(id), Some(active)) if id != active => {
                    format!("Selected rover {id}. Video is rover {active}.")
                }
                (Some(id), _) => format!("Selected rover {id}"),
                (None, Some(active)) => format!("Video is rover {active}"),
                (None, None) => "Select a rover, then start video.".into(),
            },
        ))
        .child(
            div()
                .w_full()
                .h(px(188.))
                .bg(rgb(0x0b141d))
                .rounded_md()
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
                            .text_size(px(12.))
                            .text_color(rgb(0x98aeb9))
                            .child(detail.clone()),
                    )
                }),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(phase_color(phase))
                .child(format!("{} · {detail}", phase.label())),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(0x98aeb9))
                .child(stats),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(button(
                    "fpv-start",
                    "Start FPV",
                    rgb(0x245c49),
                    cx.listener(|this, _, _, cx| {
                        if let Some(id) = this.selected {
                            this.fpv.start_phone(id);
                            this.note = format!("FPV requested for rover {id}");
                        } else {
                            this.note = "Select a rover before starting FPV".into();
                        }
                        cx.notify();
                    }),
                ))
                .child(button(
                    "fpv-stop",
                    "Stop FPV",
                    rgb(0x5c2a2a),
                    cx.listener(|this, _, _, cx| {
                        this.fpv.stop();
                        this.note = "FPV stopped".into();
                        cx.notify();
                    }),
                )),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(0x98aeb9))
                .child("Video only. Drive, arm, and stop stay on the Zenoh controls."),
        )
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

fn phase_color(phase: FpvPhase) -> Rgba {
    rgb(match phase {
        FpvPhase::Connected => 0x3dcc9a,
        FpvPhase::Failed => 0xe07a5f,
        FpvPhase::Connecting | FpvPhase::Starting => 0xf0c36a,
        FpvPhase::Idle | FpvPhase::Closed => 0x98aeb9,
    })
}

fn navigation_toggle(mark: &RoverMark, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let id = mark.id;
    let selected = mark
        .outdoor
        .as_ref()
        .map(|report| report.source.as_str())
        .unwrap_or("local");
    div().flex().gap_1().children(
        [("local", "Local"), ("outdoor_gps", "Outdoor GPS")]
            .into_iter()
            .enumerate()
            .map(|(index, (source, label))| {
                let source = source.to_string();
                let current = selected == source;
                div()
                    .id(("source", id.saturating_mul(2).saturating_add(index as u64)))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_size(px(11.))
                    .cursor_pointer()
                    .bg(rgb(if current { 0x245c49 } else { 0x1b2c38 }))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.selected == Some(id) {
                            this.feed.select_source(id, &source);
                        }
                        cx.notify();
                    }))
            }),
    )
}

fn level_line(mark: &RoverMark) -> String {
    let current = mark
        .effective_level
        .as_deref()
        .or(mark.requested_level.as_deref())
        .map(level_label)
        .unwrap_or("level unknown");
    let recommended = mark
        .recommended_level
        .as_deref()
        .map(|level| format!(" · recommended {}", level_label(level)))
        .unwrap_or_default();
    format!("{current}{recommended}")
}

fn rover_panel(
    mark: RoverMark,
    drag: Option<(u64, f64, f64, f64, Option<f64>)>,
    driving: bool,
    limits: argos_core::TeleopLimits,
    link: &str,
    grid_frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
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
    let outdoor = if mark.app_update_required {
        "Local navigation. Outdoor GPS is unavailable in this runtime.".to_string()
    } else if mark.outdoor_gps {
        "Outdoor GPS acknowledged. Drag the map marker. Release sends the WGS84 goal.".into()
    } else {
        "Local navigation. Outdoor GPS stays off until the vehicle acknowledges it.".into()
    };
    let drive_reason = drive_block(&mark.hardware);
    let hardware_blocked = drive_reason.is_some();
    let drag_line = if let Some((_, _, _, distance, radius)) = drag.filter(|item| item.0 == id) {
        match radius {
            Some(radius) => format!("{distance:.1} m GPS · arrival radius {radius:.0} m"),
            None => format!("{distance:.1} m"),
        }
    } else if mark.outdoor_gps {
        match drag_disabled_reason(geographic_gate(&mark, link, true)) {
            Some(reason) => reason.to_string(),
            None if hardware_blocked => drive_reason.unwrap().into(),
            None => "Drag the map marker. Release sends the GPS goal.".into(),
        }
    } else {
        match drag_disabled_reason(local_gate) {
            Some(reason) => reason.to_string(),
            None if hardware_blocked => drive_reason.unwrap().into(),
            None => "Drag the rover or its goal. Release sends.".into(),
        }
    };
    div()
        .flex()
        .flex_col()
        .gap_2()
        .mt_1()
        .child(div().text_size(px(12.)).child("Navigation"))
        .child(navigation_toggle(&mark, cx))
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(0x98aeb9))
                .child(outdoor.clone()),
        )
        .when(mark.app_update_required && mark.outdoor_gps, |panel| {
            panel.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(0xf0c36a))
                    .child(APP_UPDATE_REQUIRED),
            )
        })
        .when_some(mark.raw_gps, |panel, gps| {
            panel.child(div().text_size(px(12.)).child(format!(
                    "{} · {:.5}, {:.5}{}{}",
                    position_label(&mark),
                    gps.latitude,
                    gps.longitude,
                    gps.accuracy_m
                        .map(|accuracy| format!(" · ±{accuracy:.1} m"))
                        .unwrap_or_default(),
                    gps.age_s
                        .map(|age| format!(" · {age:.1} s"))
                        .unwrap_or_default()
                )))
        })
        .when_some(
            mark.outdoor.as_ref().and_then(|report| report.hold_label()),
            |panel, label| {
                panel.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(0xe7b15a))
                        .child(label),
                )
            },
        )
        .when_some(mark.outdoor.clone(), |panel, report| {
            let remaining = report.remaining_m.map(|metres| format!("{metres:.0} m"));
            let destination = match (report.destination_latitude, report.destination_longitude) {
                (Some(latitude), Some(longitude)) => Some(format!("{latitude:.5}, {longitude:.5}")),
                _ => None,
            };
            panel
                .child(
                    div()
                        .text_size(px(12.))
                        .child(format!("Phase {}", phase_label(&report.phase))),
                )
                .when_some(remaining, |panel, remaining| {
                    panel.child(
                        div()
                            .text_size(px(12.))
                            .child(format!("Remaining {remaining}")),
                    )
                })
                .when_some(destination, |panel, destination| {
                    panel.child(
                        div()
                            .text_size(px(12.))
                            .child(format!("Destination {destination}")),
                    )
                })
                .child(
                    div()
                        .text_size(px(12.))
                        .child(format!("Arrival radius {:.0} m", report.arrival_radius_m)),
                )
        })
        .child(div().text_size(px(12.)).child("Autonomy"))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(
                    OPERATOR_LEVELS
                        .iter()
                        .enumerate()
                        .map(|(index, (wire, label))| {
                            let wire = (*wire).to_string();
                            let current = mark.requested_level.as_deref() == Some(wire.as_str());
                            div()
                                .id(("level", index as u64))
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .text_size(px(11.))
                                .cursor_pointer()
                                .bg(rgb(if current { 0x245c49 } else { 0x1b2c38 }))
                                .child(*label)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.selected == Some(id) {
                                        this.feed.publish_level(id, &wire);
                                        this.note = format!("Sending {}", level_label(&wire));
                                    }
                                    cx.notify();
                                }))
                        }),
                ),
        )
        .when(mark.hold.as_deref() == Some(HEADING_HOLD), |panel| {
            panel.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(0xe7b15a))
                    .child(HEADING_HOLD_LABEL),
            )
        })
        .child(div().text_size(px(12.)).child("Local map"))
        .child(local_grid(&mark, drag, grid_frame, cx))
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(0xf0c36a))
                .child(drag_line),
        )
        .when(
            mark.route
                .iter()
                .any(|sample| sample.travel == Travel::Unspecified)
                && mark.route.len() > 1,
            |panel| {
                panel.child(
                    div().text_size(px(11.)).text_color(rgb(0x98aeb9)).child(
                        "Path direction is provisional until a segment velocity is published.",
                    ),
                )
            },
        )
        .when(
            mark.outdoor.as_ref().is_some_and(|r| r.phase == "paused"),
            |panel| {
                panel.child(button(
                    "resume-outdoor",
                    "Resume outdoor mission",
                    rgb(0x24594a),
                    cx.listener(move |this, _, _, cx| {
                        this.feed.resume_outdoor(id);
                        cx.notify();
                    }),
                ))
            },
        )
        .child(button(
            "cancel-goal",
            "Cancel goal",
            rgb(0x1b2c38),
            cx.listener(move |this, _, _, cx| {
                this.feed.cancel_goal(id);
                this.drag = None;
                this.note = "Cancel requested".into();
                cx.notify();
            }),
        ))
        .child(
            div()
                .text_size(px(12.))
                .child(if driving { "Drive · manual" } else { "Drive" }),
        )
        .when(hardware_blocked && !driving, |panel| {
            panel.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(0xf0c36a))
                    .child(drive_reason.unwrap_or_default().to_string()),
            )
        })
        .child(
            div().flex().gap_2().flex_wrap().children(
                [0.1, 0.2, 0.3, 0.4, 0.5]
                    .into_iter()
                    .enumerate()
                    .map(|(index, speed)| {
                        let chosen = (limits.linear - speed).abs() < 0.001;
                        div()
                            .id(("speed", index as u64))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_size(px(11.))
                            .cursor_pointer()
                            .bg(rgb(if chosen { 0x245c49 } else { 0x1b2c38 }))
                            .child(format!("{speed:.1} m/s"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Err(error) = this.feed.set_linear(speed) {
                                    this.note = error;
                                }
                                cx.notify();
                            }))
                    }),
            ),
        )
        .when(
            mark.requested_level.as_deref() == Some("teleop") && mark.status == "pose stale",
            |panel| {
                panel.child(
                    div()
                        .text_size(px(12.))
                        .child("Camera pose unavailable · manual control does not need tracking"),
                )
            },
        )
        .child(turning_row(limits, cx))
        .child(
            div().text_size(px(11.)).text_color(rgb(0x98aeb9)).child(
                "Hold W/A/S/D to drive. Release to stop. Reverse uses the same turning radius.",
            ),
        )
        .child(drive_pad(driving, cx))
        .child(
            div()
                .flex()
                .gap_2()
                .when(driving, |this| {
                    this.child(button(
                        "leave-drive",
                        "Leave drive",
                        rgb(0x1b2c38),
                        cx.listener(|this, _, _, cx| {
                            this.stop_drive();
                            this.note = "Drive off. Zero twist sent.".into();
                            cx.notify();
                        }),
                    ))
                })
                .when(!driving, |this| {
                    this.child(button(
                        "start-drive",
                        if mark.requested_level.as_deref() == Some("teleop") {
                            "Drive this rover"
                        } else {
                            "Select manual control"
                        },
                        rgb(0x245c49),
                        cx.listener(|this, _, window, cx| {
                            this.begin_drive(window, cx);
                            cx.notify();
                        }),
                    ))
                })
                .child(button(
                    "estop",
                    "Stop",
                    rgb(0x8d2b2b),
                    cx.listener(|this, _, _, cx| {
                        this.estop();
                        cx.notify();
                    }),
                )),
        )
}

fn turning_row(limits: argos_core::TeleopLimits, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let yaw = cycle(limits.yaw_rate, &[0.25, 0.5, 1.0]);
    let accel = cycle(limits.yaw_accel, &[0.25, 0.5, 1.0]);
    let decel = cycle(limits.yaw_decel, &[0.4, 0.8, 1.6]);
    let radius = cycle(limits.min_turn_radius, &[0.35, 0.5, 1.0]);
    div()
        .flex()
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
        .child(button(
            "spin",
            if limits.allow_spin {
                "In-place spin on"
            } else {
                "In-place spin off"
            },
            rgb(if limits.allow_spin {
                0x8a5a12
            } else {
                0x1b2c38
            }),
            cx.listener(|this, _, _, cx| {
                let allow = !this.feed.limits().allow_spin;
                if let Err(error) = this.feed.set_spin(allow) {
                    this.note = error;
                }
                cx.notify();
            }),
        ))
}

fn limit_chip(
    id: &'static str,
    label: String,
    cx: &mut Context<Dashboard>,
    apply: impl Fn(&mut Dashboard) -> Result<(), String> + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_md()
        .text_size(px(11.))
        .cursor_pointer()
        .bg(rgb(0x1b2c38))
        .child(label)
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

fn drive_pad(driving: bool, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let key = |name: &'static str, label: &'static str, cx: &mut Context<Dashboard>| {
        div()
            .id(name)
            .w(px(36.))
            .h(px(28.))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(if driving { 0x245c49 } else { 0x1b2c38 }))
            .text_size(px(12.))
            .child(label)
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
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().flex().justify_center().child(key("w", "W", cx)))
        .child(
            div()
                .flex()
                .justify_center()
                .gap_1()
                .child(key("a", "A", cx))
                .child(key("s", "S", cx))
                .child(key("d", "D", cx)),
        )
}

fn hardware_row(id: u64, cx: &mut Context<Dashboard>) -> impl IntoElement {
    div()
        .flex()
        .gap_1()
        .flex_wrap()
        .child(
            Button::new(("stop-rover", id))
                .label("Stop")
                .danger()
                .compact()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.stop_rover(id);
                    cx.stop_propagation();
                    cx.notify();
                })),
        )
        .child(
            Button::new(("reset-stop", id))
                .label("Reset stop")
                .compact()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.reset_safety(id);
                    cx.stop_propagation();
                    cx.notify();
                })),
        )
}

fn button(
    id: &'static str,
    label: &'static str,
    color: Rgba,
    listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_3()
        .py_2()
        .rounded_md()
        .cursor_pointer()
        .bg(color)
        .text_size(px(13.))
        .child(label)
        .on_click(listener)
}

/// Window chrome for the console: map camera, link health, and the fleet stop.
fn console_bar(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let mode = dashboard.map_mode;
    TitleBar::new()
        .child(
            h_flex()
                .gap_3()
                .child(div().text_sm().font_semibold().child("ARGOS"))
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
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .pr_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
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

/// Rovers whose position is outside the current view. Choosing one centers
/// the map on it and selects it.
fn offscreen_tabs(
    field: &[(u64, Option<Point<Pixels>>, console::Callout)],
    selected: Option<u64>,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    v_flex()
        .absolute()
        .right_3()
        .bottom_3()
        .items_end()
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
}

/// The selected rover's state and the commands that act on it. Arm, levels,
/// navigation source, and drive limits stay in the sidebar for now.
fn rover_inspector(
    mark: &RoverMark,
    stopped: bool,
    driving: bool,
    frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let id = mark.id;
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
    v_flex()
        .absolute()
        .top_3()
        .right_3()
        .w_80()
        .p_4()
        .gap_3()
        .popover_style(cx)
        // Measured so rover cards can stay out from under the inspector.
        .child(
            canvas(
                move |bounds, _, _| *frame.lock().unwrap() = Some(bounds),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
                .children(callout.tag.map(callout_tag)),
        )
        .child(div().text_base().font_medium().child(callout.state))
        .child(
            DescriptionList::horizontal()
                .columns(1)
                .bordered(false)
                .label_width(rems(6.))
                .children(
                    console::trace(mark, stopped)
                        .into_iter()
                        .map(|(label, value)| DescriptionItem::new(label).value(value)),
                ),
        )
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
        )
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
    let mode = match argos_gpui::LaunchMode::parse(std::env::args().skip(1)) {
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
                    title: Some(
                        match mode {
                            argos_gpui::LaunchMode::Sim => "ARGOS · Simulation",
                            argos_gpui::LaunchMode::Real => "ARGOS · Real fleet",
                        }
                        .into(),
                    ),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
            },
            cx,
            |window, cx| {
                // The field and every console surface are drawn for a dark room.
                Theme::change(ThemeMode::Dark, Some(window), cx);
                let view = cx.new(|cx| Dashboard::new(map, feed, fpv, maps_notice, cx));
                let focus = view.read(cx).focus.clone();
                focus.focus(window, cx);
                view
            },
        )
        .expect("Failed to open window");
        cx.activate(true);
    });
}
