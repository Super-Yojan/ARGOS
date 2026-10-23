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
    APP_UPDATE_REQUIRED, ArmControl, ArmGate, ControlClaim, DragAnchor, DragGate, DragSpace,
    GoalDrag, HEADING_HOLD, HEADING_HOLD_LABEL, HardwareCommand, OPERATOR_LEVELS, PlanarPoint,
    Travel, arm_disabled_reason, drag_disabled_reason, drive_block, level_label, phase_label,
    pose_warning_text,
};
use argos_gpui::{
    DashboardConfig, FleetFeed, FpvController, FpvPhase, RoverMark, model_position,
    reframing_target,
};
use gpui_kit::{prelude::*, *};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use terra_map::{Color, GeoPosition, Map, MapConfig, MapEvent, MapView, ObjectId};

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
    selected: Option<u64>,
    drag: Option<ActiveDrag>,
    grid_frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
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
    arms: BTreeMap<u64, ArmControl>,
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
            selected: None,
            drag: None,
            grid_frame: Arc::new(Mutex::new(None)),
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
            arms: BTreeMap::new(),
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
        self.tick_arms();
        let seen: BTreeSet<u64> = self.marks.iter().map(|mark| mark.id).collect();
        self.arms.retain(|id, _| seen.contains(id));
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
    fn tick_arms(&mut self) {
        let now = self.started.elapsed().as_secs_f64();
        let dt = (now - self.now).max(0.);
        self.now = now;
        self.claim.renew(now);
        let samples: Vec<_> = self
            .marks
            .iter()
            .map(|mark| {
                (
                    mark.id,
                    mark.run_id.clone(),
                    mark.authority_revision,
                    mark.hardware.clone(),
                    self.selected == Some(mark.id),
                    mark.control_pose_ready(),
                    self.latched.contains(&mark.id),
                    mark.safety.clone(),
                    !mark.run_id.is_empty(),
                    mark.hardware.present,
                )
            })
            .collect();
        let link_healthy = self.link == "healthy";
        let claim_held = self.claim.held(self.now);
        let mut commands = Vec::new();
        for (
            id,
            run_id,
            revision,
            hardware,
            selected,
            pose_fresh,
            local_latch,
            safety,
            run_ready,
            hardware_present,
        ) in &samples
        {
            let gate = ArmGate {
                selected: *selected,
                link_healthy,
                pose_fresh: *pose_fresh,
                local_latch: *local_latch,
                safety,
                claim_held,
                run_ready: *run_ready,
                hardware_present: *hardware_present,
            };
            if let Some(command) = self
                .arms
                .entry(*id)
                .or_default()
                .tick(dt, gate, hardware, run_id, *revision)
            {
                commands.push((*id, command));
            }
        }
        for (id, command) in commands {
            self.dispatch_hardware(id, command);
        }
    }
    fn arm_gate<'a>(&self, mark: &'a RoverMark) -> ArmGate<'a> {
        ArmGate {
            selected: self.selected == Some(mark.id),
            link_healthy: self.link == "healthy",
            pose_fresh: mark.control_pose_ready(),
            local_latch: self.latched.contains(&mark.id),
            safety: &mark.safety,
            claim_held: self.claim.held(self.now),
            run_ready: !mark.run_id.is_empty(),
            hardware_present: mark.hardware.present,
        }
    }
    fn dispatch_hardware(&mut self, id: u64, command: HardwareCommand) {
        match command {
            HardwareCommand::Reset => {
                if let Err(error) = self.feed.safety_reset(id) {
                    self.note = error;
                }
            }
            command => self.feed.publish_hardware(id, command),
        }
    }
    fn arm_press(&mut self, id: u64) {
        if self.selected != Some(id) {
            self.select(id);
            self.note = "Select this rover before arming".into();
            return;
        }
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id).cloned() else {
            return;
        };
        let gate = self.arm_gate(&mark);
        if let Some(reason) = self.arms.entry(id).or_default().press(gate) {
            self.note = reason.into();
        } else {
            self.note = "Hold to arm".into();
        }
    }
    fn arm_release(&mut self, id: u64) {
        let Some(control) = self.arms.get_mut(&id) else {
            return;
        };
        let was_holding = control.holding().is_some();
        control.release();
        if was_holding && self.note == "Hold to arm" {
            self.note.clear();
        }
    }
    fn disarm(&mut self, id: u64) {
        let run_id = self
            .marks
            .iter()
            .find(|mark| mark.id == id)
            .map(|mark| mark.run_id.clone())
            .unwrap_or_default();
        let command = self.arms.entry(id).or_default().disarm(&run_id);
        self.dispatch_hardware(id, command);
        self.note.clear();
    }
    fn reset_safety(&mut self, id: u64) {
        self.latched.remove(&id);
        let command = self.arms.entry(id).or_default().reset();
        self.dispatch_hardware(id, command);
        self.note = format!("Reset sent to rover {id}");
    }
    fn begin_drive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            self.note = "Select a rover before driving".into();
            return;
        };
        self.drag = None;
        // Mode selection does not move or arm the rover. Permit the operator to
        // leave a held navigation mode before asking for arm/resume readiness.
        if self
            .selected_mark()
            .is_some_and(|mark| mark.requested_level.as_deref() != Some("teleop"))
        {
            self.feed.publish_level(id, "teleop");
            self.note = "Selecting manual control · wait for L0 confirmation, then arm or resume if required".into();
            return;
        }
        match self.feed.begin_drive(id) {
            Ok(()) => {
                self.driving = true;
                self.note = "Drive is manual. In-place spin is off.".into();
                self.feed.cancel_goal(id);
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
            self.latched.insert(id);
            if let Some(control) = self.arms.get_mut(&id) {
                control.cancel_hold();
            }
            match self.feed.emergency_stop(id) {
                Ok(()) => self.note = format!("Stop latched for rover {id}"),
                Err(error) => self.note = error,
            }
        }
        self.driving = false;
        self.held.clear();
    }
    fn hold(&mut self, key: &str, down: bool) {
        let Some(id) = self.selected else {
            return;
        };
        if !self.driving {
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
        div()
            .id("argos")
            .track_focus(&self.focus)
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                if let Some(key) = drive_key(&event.keystroke.key) {
                    this.hold(key, false);
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.cancel_drag();
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.key == "space" {
                    this.estop();
                    cx.stop_propagation();
                    return;
                }
                if this.driving
                    && let Some(key) = drive_key(&event.keystroke.key)
                {
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
            .bg(rgb(0x0b141d))
            .text_color(rgb(0xd8e7ec))
            .font_family(".SystemUIFont")
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
                    .children(self.marks.iter().map(|mark| {
                        let id = mark.id;
                        let attention = mark.status != "online";
                        let chosen = selected == Some(id);
                        let face = arm_face(self.arms.get(&id), mark, self.arm_gate(mark));
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
                                mark.status, mark.occupancy
                            ))
                            .child(div().text_color(rgb(0x98aeb9)).child(level_line(mark)))
                            .child(
                                div()
                                    .text_color(rgb(0xf0c36a))
                                    .child(face.status.clone()),
                            )
                            .child(hardware_row(id, &face, cx))
                            .when(!face.detail.is_empty(), |row| {
                                let detail = face.detail.clone();
                                row.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(0xf0c36a))
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
                                "No rover position yet. The map stays at the start location.",
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
                            "Control-drag a 3D rover to place its destination; release to send. The ground marker stays at its live position. Outdoor GPS sends WGS84. Esc cancels. Hold Arm for one second. Disarm is one click. Reset clears a latched stop and does not arm.",
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
                    })),
            )
            .child(fpv_panel(self, cx))
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
    let needs_arm = drive_reason.is_some();
    let drag_line = if let Some((_, _, _, distance, radius)) = drag.filter(|item| item.0 == id) {
        match radius {
            Some(radius) => format!("{distance:.1} m GPS · arrival radius {radius:.0} m"),
            None => format!("{distance:.1} m"),
        }
    } else if mark.outdoor_gps {
        match drag_disabled_reason(geographic_gate(&mark, link, true)) {
            Some(reason) => reason.to_string(),
            None if needs_arm => drive_reason.unwrap().into(),
            None => "Drag the map marker. Release sends the GPS goal.".into(),
        }
    } else {
        match drag_disabled_reason(local_gate) {
            Some(reason) => reason.to_string(),
            None if needs_arm => drive_reason.unwrap().into(),
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
                    "GPS {:.5}, {:.5}{}{}",
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
        .when(needs_arm && !driving, |panel| {
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

struct ArmFace {
    status: String,
    detail: String,
    arm_label: &'static str,
    arm_enabled: bool,
}

fn arm_face(control: Option<&ArmControl>, mark: &RoverMark, gate: ArmGate<'_>) -> ArmFace {
    let control = control.cloned().unwrap_or_default();
    let reason = arm_disabled_reason(gate);
    let armed = mark.hardware.present && mark.hardware.armed;
    let arm_enabled = reason.is_none()
        && !control.pending_arm()
        && (!armed || mark.hardware.motion_paused())
        && !mark.hardware.arming;
    let arm_label = if control.holding().is_some() {
        "Hold…"
    } else if control.pending_arm() || (mark.hardware.arming && !armed) {
        "arming…"
    } else if mark.hardware.motion_paused() {
        "Resume motion"
    } else {
        "Arm"
    };
    let detail = if let Some(error) = control.error.clone() {
        error
    } else if control.holding().is_some() {
        String::new()
    } else if !arm_enabled && !control.pending_arm() {
        reason.unwrap_or("").to_string()
    } else if arm_enabled {
        if mark.hardware.motion_paused() {
            format!(
                "{} · Hold Resume motion; previous input is cleared",
                drive_block(&mark.hardware).unwrap_or("Motion paused")
            )
        } else {
            "Hold to arm".into()
        }
    } else {
        String::new()
    };
    ArmFace {
        status: control.status_label(&mark.hardware),
        detail,
        arm_label,
        arm_enabled,
    }
}

fn hardware_row(id: u64, face: &ArmFace, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let enabled = face.arm_enabled;
    let arm_label = face.arm_label;
    div()
        .flex()
        .gap_1()
        .flex_wrap()
        .child(
            div()
                .id(("arm", id))
                .px_2()
                .py_1()
                .rounded_md()
                .text_size(px(12.))
                .bg(rgb(if enabled { 0x8a5a12 } else { 0x1b2c38 }))
                .text_color(rgb(if enabled { 0xf8efe2 } else { 0x98aeb9 }))
                .when(enabled, |this| {
                    this.cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.arm_press(id);
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        )
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.arm_release(id);
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        )
                        .on_mouse_up_out(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.arm_release(id);
                                cx.notify();
                            }),
                        )
                })
                .child(arm_label),
        )
        .child(card_button(
            "Disarm",
            id.saturating_mul(2),
            rgb(0x1b2c38),
            cx,
            move |this| {
                this.disarm(id);
            },
        ))
        .child(card_button(
            "Reset",
            id.saturating_mul(2).saturating_add(1),
            rgb(0x1b2c38),
            cx,
            move |this| {
                this.reset_safety(id);
            },
        ))
}

fn card_button(
    label: &'static str,
    slot: u64,
    color: Rgba,
    cx: &mut Context<Dashboard>,
    apply: impl Fn(&mut Dashboard) + 'static,
) -> impl IntoElement {
    div()
        .id(("hardware", slot))
        .px_2()
        .py_1()
        .rounded_md()
        .text_size(px(12.))
        .cursor_pointer()
        .bg(color)
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                apply(this);
                cx.stop_propagation();
                cx.notify();
            }),
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
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(12.), px(12.))),
                }),
                ..Default::default()
            },
            cx,
            |window, cx| {
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
