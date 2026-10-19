//! ARGOS operator dashboard.
//!
//! The camera starts at `ARGOS_MAP_START_LAT` / `ARGOS_MAP_START_LON` until rover
//! positions arrive, then recenters on the fleet. `MAPS_DEMO=1` still draws the
//! offline fixture. Dragging a selected rover's marker sends a goal on release.
//! Esc, or dropping the ghost back on the rover, sends nothing. An acknowledged
//! outdoor GPS rover uses the same gesture on the map: the ghost shows the
//! arrival radius and GPS distance, and release publishes the WGS84 point.
//! That release is the explicit operator action. The outdoor GPS spec's
//! separate confirm step is not used.

use argos_core::{
    APP_UPDATE_REQUIRED, CellClass, DragAnchor, DragEnd, DragGate, DragSpace, GoalDrag,
    HEADING_HOLD, HEADING_HOLD_LABEL, OPERATOR_LEVELS, OutdoorAdmission, PlanarPoint, Travel,
    drag_disabled_reason, geodesic_metres, level_label, phase_label, pose_warning_text,
};
use argos_gpui::{DashboardConfig, FleetFeed, RoverMark, framing_target};
use gpui_kit::{prelude::*, *};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use terra_map::{Color, GeoPosition, Map, MapConfig, MapEvent, MapView, ObjectId};

const MARKER_ALTITUDE: f64 = 80.;
const MARKER_SCALE: f64 = 12.;

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
    close_preview: bool,
    _poll: Task<()>,
}
impl Dashboard {
    fn new(map: Map, feed: FleetFeed, maps_notice: Option<String>, cx: &mut Context<Self>) -> Self {
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
                        this.sync();
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
            close_preview: false,
            _poll: poll,
        }
    }
    fn sync(&mut self) {
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
        let seen: BTreeSet<u64> = self.marks.iter().map(|mark| mark.id).collect();
        self.sync_markers(&seen, false);
        self.sync_markers(&seen, true);
        if seen.is_empty() {
            self.framed.clear();
        } else if let Some((ids, latitude, longitude)) = framing_target(&self.framed, &self.marks)
            && self.map.show_lat_lon(latitude, longitude).is_ok()
        {
            self.framed = ids;
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
    fn sync_markers(&mut self, seen: &BTreeSet<u64>, goals: bool) {
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
                (latitude, longitude, 8., Some(Color::rgb(40, 200, 140)))
            } else {
                (
                    mark.latitude,
                    mark.longitude,
                    MARKER_SCALE,
                    (mark.status != "online").then_some(Color::rgb(255, 160, 50)),
                )
            };
            let position = GeoPosition::new(latitude, longitude, MARKER_ALTITUDE);
            let object = if let Some(object) = objects.get(&mark.id).copied() {
                if self.map.set_object_position(object, position).is_err() {
                    continue;
                }
                object
            } else {
                match self.map.add_3d_obj(position, &self.model) {
                    Ok(object) => {
                        let _ = self.map.set_object_scale(object, scale);
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
        if self.selected != Some(id) {
            self.stop_drive();
            self.drag = None;
            self.selected = Some(id);
        }
    }
    fn begin_drive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            self.note = "Select a rover before driving".into();
            return;
        };
        self.drag = None;
        match self.feed.begin_drive(id) {
            Ok(()) => {
                self.driving = true;
                self.note = "Drive is manual. In-place spin is off.".into();
                self.feed.publish_level(id, "teleop");
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
    fn begin_drag(&mut self, id: u64, anchor: DragAnchor, from_map: bool) {
        if self.driving {
            self.note = "Leave drive before dragging a goal".into();
            return;
        }
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id).cloned() else {
            return;
        };
        if !from_map && mark.outdoor_gps {
            self.note =
                "Outdoor GPS is the source. Drag the map marker to send a WGS84 goal.".into();
            return;
        }
        let geographic = from_map && mark.outdoor_gps;
        let level = mark
            .effective_level
            .clone()
            .or_else(|| mark.requested_level.clone());
        let gate = if geographic {
            geographic_gate(&mark, &self.link, self.selected == Some(id))
        } else {
            DragGate {
                selected: self.selected == Some(id),
                link_healthy: self.link == "healthy",
                pose_fresh: mark.status == "online",
                degraded: mark.degraded,
                level: level.as_deref(),
                geographic: false,
                outdoor: false,
                admission: None,
            }
        };
        let (space, rover, cursor, radius) = if geographic {
            let rover = gps_point(&mark);
            let cursor = if anchor == DragAnchor::Goal {
                goal_geo(&mark).unwrap_or(rover)
            } else {
                rover
            };
            (
                DragSpace::Geographic,
                rover,
                cursor,
                Some(mark.arrival_radius),
            )
        } else {
            let rover = PlanarPoint {
                x: mark.local_x,
                y: mark.local_y,
            };
            let cursor = if anchor == DragAnchor::Goal {
                mark.goal
                    .as_ref()
                    .map(|goal| PlanarPoint {
                        x: goal.x,
                        y: goal.y,
                    })
                    .unwrap_or(rover)
            } else {
                rover
            };
            (DragSpace::Local, rover, cursor, None)
        };
        match GoalDrag::start(id, anchor, space, rover, cursor, radius, gate) {
            Ok(goal) => {
                self.note = drag_note(&goal);
                self.drag = Some(ActiveDrag { goal, from_map });
            }
            Err(reason) => self.note = reason.into(),
        }
    }
    fn move_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(active) = self.drag.as_ref() else {
            return;
        };
        let from_map = active.from_map;
        let space = active.goal.space;
        let next = if from_map {
            let Some((latitude, longitude)) = self.view.read(cx).ground_at(position) else {
                return;
            };
            if space == DragSpace::Geographic {
                PlanarPoint {
                    x: latitude,
                    y: longitude,
                }
            } else {
                let Ok((x, y)) = self.feed.origin().local(latitude, longitude) else {
                    return;
                };
                PlanarPoint { x, y }
            }
        } else {
            let frame = *self.grid_frame.lock().unwrap();
            let Some(frame) = frame else {
                return;
            };
            let Some(grid) = self.selected_mark().and_then(|mark| mark.grid.clone()) else {
                return;
            };
            let Some((x, y)) = point_to_local(&frame, &grid, position) else {
                return;
            };
            PlanarPoint { x, y }
        };
        let Some(active) = self.drag.as_mut() else {
            return;
        };
        active.goal.move_to(next);
        self.note = drag_note(&active.goal);
    }
    fn release_drag(&mut self) {
        let Some(active) = self.drag.take() else {
            return;
        };
        let id = active.goal.id;
        let radius = active.goal.arrival_radius_m;
        let geographic = active.goal.space == DragSpace::Geographic;
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id).cloned() else {
            self.note = "Drag cancelled".into();
            return;
        };
        let level = mark
            .effective_level
            .clone()
            .or_else(|| mark.requested_level.clone());
        let gate = if geographic {
            geographic_gate(&mark, &self.link, self.selected == Some(id))
        } else {
            DragGate {
                selected: self.selected == Some(id),
                link_healthy: self.link == "healthy",
                pose_fresh: mark.status == "online",
                degraded: mark.degraded,
                level: level.as_deref(),
                geographic: false,
                outdoor: false,
                admission: None,
            }
        };
        match active.goal.release(gate) {
            DragEnd::Cancelled(reason) => self.note = reason.into(),
            DragEnd::Send(goal) => {
                self.feed.publish_goal(id, goal, radius);
                self.note = match radius {
                    Some(radius) => format!("Goal sent. Arrival radius frozen at {radius:.0} m"),
                    None => "Goal sent".into(),
                };
            }
        }
    }
    fn cancel_drag(&mut self) {
        if let Some(active) = self.drag.take() {
            self.note = match active.goal.escape() {
                DragEnd::Cancelled(reason) => reason.into(),
                DragEnd::Send(_) => "Drag cancelled".into(),
            };
        }
    }
}
impl Render for Dashboard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.driving && !window.is_window_active() {
            self.stop_drive();
            self.note = "Drive stopped because the window lost focus".into();
        }
        let selected = self.selected;
        let driving = self.driving;
        let limits = self.feed.limits();
        let mark = self.selected_mark().cloned();
        let drag_local = self.drag.as_ref().and_then(|active| {
            let goal = &active.goal;
            let local = match goal.space {
                DragSpace::Local => Some((goal.cursor.x, goal.cursor.y)),
                DragSpace::Geographic => {
                    self.feed.origin().local(goal.cursor.x, goal.cursor.y).ok()
                }
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
                    .children(self.marks.iter().map(|mark| {
                        let id = mark.id;
                        let attention = mark.status != "online";
                        let chosen = selected == Some(id);
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
                            "Drag a marker and release to send. Outdoor GPS sends WGS84 from the map marker. Esc cancels. The map grid is the local occupancy draped at the rover.",
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
                            .bg(rgb(if anchor == DragAnchor::Goal {
                                0x3dcc9a
                            } else {
                                0xf0b429
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.begin_drag(id, anchor, true);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                    })),
            )
    }
}

fn gps_point(mark: &RoverMark) -> PlanarPoint {
    if let Some(gps) = mark.raw_gps {
        return PlanarPoint {
            x: gps.latitude,
            y: gps.longitude,
        };
    }
    if let Some(report) = &mark.outdoor
        && let (Some(latitude), Some(longitude)) = (report.gps_latitude, report.gps_longitude)
    {
        return PlanarPoint {
            x: latitude,
            y: longitude,
        };
    }
    PlanarPoint {
        x: mark.latitude,
        y: mark.longitude,
    }
}

fn geographic_gate<'a>(mark: &'a RoverMark, link: &str, selected: bool) -> DragGate<'a> {
    let level = mark
        .effective_level
        .as_deref()
        .or(mark.requested_level.as_deref());
    let point = gps_point(mark);
    let radius = mark.arrival_radius.clamp(2., 20.);
    DragGate {
        selected,
        link_healthy: link == "healthy",
        pose_fresh: mark.status == "online",
        degraded: mark.degraded,
        level,
        geographic: true,
        outdoor: mark.outdoor_gps,
        admission: Some(OutdoorAdmission {
            report: mark.outdoor.as_ref(),
            latitude: point.x,
            longitude: point.y,
            arrival_radius_m: radius,
            level,
            run_id: &mark.run_id,
            navigation_revision: mark.navigation_revision,
        }),
    }
}

fn drag_note(goal: &GoalDrag) -> String {
    match goal.arrival_radius_m {
        Some(radius) => format!(
            "{:.1} m GPS · arrival radius {radius:.0} m — release sends, Esc cancels",
            goal.distance_m()
        ),
        None => format!("{:.1} m — release sends, Esc cancels", goal.distance_m()),
    }
}

fn goal_geo(mark: &RoverMark) -> Option<PlanarPoint> {
    let goal = mark.goal.as_ref()?;
    Some(PlanarPoint {
        x: goal.latitude?,
        y: goal.longitude?,
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
        APP_UPDATE_REQUIRED.to_string()
    } else if mark.outdoor_gps {
        "Outdoor GPS acknowledged. Drag the map marker. Release sends the WGS84 goal.".into()
    } else {
        "Local navigation. Outdoor GPS stays off until the vehicle acknowledges it.".into()
    };
    let drag_line = if let Some((_, _, _, distance, radius)) = drag.filter(|item| item.0 == id) {
        match radius {
            Some(radius) => format!("{distance:.1} m GPS · arrival radius {radius:.0} m"),
            None => format!("{distance:.1} m"),
        }
    } else if mark.outdoor_gps {
        match drag_disabled_reason(geographic_gate(&mark, link, true)) {
            Some(reason) => reason.to_string(),
            None => "Drag the map marker. Release sends the GPS goal.".into(),
        }
    } else {
        match drag_disabled_reason(local_gate) {
            Some(reason) => reason.to_string(),
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
        .when(mark.app_update_required, |panel| {
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
        .child(turning_row(limits, cx))
        .child(div().text_size(px(11.)).text_color(rgb(0x98aeb9)).child(
            "Defense in depth; Terra enforces the real limits. Reverse uses the same radius.",
        ))
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
                        "Drive this rover",
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

fn local_grid(
    mark: &RoverMark,
    drag: Option<(u64, f64, f64, f64, Option<f64>)>,
    grid_frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let Some(map) = mark.grid.clone() else {
        return div()
            .text_size(px(12.))
            .text_color(rgb(0x98aeb9))
            .child("No occupancy yet")
            .into_any_element();
    };
    let cols = map.cols;
    let rows = map.rows;
    let cell = (248. / cols.max(1) as f32).clamp(6., 16.);
    let width = cell * cols as f32;
    let height = cell * rows as f32;
    let id = mark.id;
    let pose = (mark.local_x, mark.local_y);
    let route = mark.route.clone();
    let goal = mark.goal.as_ref().map(|goal| (goal.x, goal.y));
    let warn =
        pose_warning_text(mark.command_origin, mark.degraded, &mark.degrade_reason).is_some();
    let drag_here = drag
        .filter(|item| item.0 == id)
        .map(|item| (item.1, item.2, item.4));
    let rover_at = local_pixel(width, height, &map, pose.0, pose.1);
    let goal_at = goal.map(|(x, y)| local_pixel(width, height, &map, x, y));
    div()
        .relative()
        .w(px(width))
        .h(px(height))
        .child(div().flex().flex_col().children((0..rows).map(|visual| {
            let row = rows - 1 - visual;
            div().flex().h(px(cell)).children((0..cols).map(|col| {
                let index = (row * cols + col) as usize;
                let class = map.cells[index];
                div()
                    .id(("cell", id.saturating_mul(1000).saturating_add(index as u64)))
                    .w(px(cell))
                    .h(px(cell))
                    .bg(rgb(cell_color(class)))
            }))
        })))
        .child(grid_overlay(
            map, pose, route, goal, drag_here, warn, grid_frame,
        ))
        .child(drag_handle_at(id, DragAnchor::Rover, rover_at, false, cx))
        .when_some(goal_at, |grid, at| {
            grid.child(drag_handle_at(id, DragAnchor::Goal, at, false, cx))
        })
        .when(mark.hold.as_deref() == Some(HEADING_HOLD), |grid| {
            grid.child(
                div()
                    .absolute()
                    .bottom(px(2.))
                    .left(px(2.))
                    .text_size(px(10.))
                    .text_color(rgb(0xe7b15a))
                    .child(HEADING_HOLD_LABEL),
            )
        })
        .into_any_element()
}

fn cell_color(class: CellClass) -> u32 {
    match class {
        CellClass::Unknown => 0x6e7c84,
        CellClass::Free => 0xd7efe0,
        CellClass::Occupied => 0x12161a,
    }
}

fn grid_overlay(
    map: argos_core::LocalMap,
    pose: (f64, f64),
    route: Vec<argos_core::RouteSample>,
    goal: Option<(f64, f64)>,
    drag: Option<(f64, f64, Option<f64>)>,
    warn: bool,
    frames: Arc<Mutex<Option<Bounds<Pixels>>>>,
) -> impl IntoElement {
    canvas(
        move |bounds, _, _| {
            *frames.lock().unwrap() = Some(bounds);
            bounds
        },
        move |bounds, _, window, _| {
            for pair in route.windows(2) {
                let travel = pair[1].travel;
                let color = if travel == Travel::Reverse {
                    rgb(0xf0c36a)
                } else {
                    rgb(0x7eb6ff)
                };
                stroke(
                    window,
                    local_to_bounds(&bounds, &map, pair[0].x, pair[0].y),
                    local_to_bounds(&bounds, &map, pair[1].x, pair[1].y),
                    color,
                    travel == Travel::Reverse,
                );
            }
            if let Some((x, y)) = goal {
                paint_dot(
                    window,
                    local_to_bounds(&bounds, &map, x, y),
                    8.,
                    rgb(0x3dcc9a),
                );
            }
            if let Some((x, y, radius)) = drag {
                stroke(
                    window,
                    local_to_bounds(&bounds, &map, pose.0, pose.1),
                    local_to_bounds(&bounds, &map, x, y),
                    rgb(0xf0c36a),
                    false,
                );
                if let Some(radius) = radius {
                    for step in 0..16 {
                        let angle = step as f64 / 16. * std::f64::consts::TAU;
                        paint_dot(
                            window,
                            local_to_bounds(
                                &bounds,
                                &map,
                                x + radius * angle.cos(),
                                y + radius * angle.sin(),
                            ),
                            3.,
                            rgb(0xf0c36a),
                        );
                    }
                }
                paint_dot(
                    window,
                    local_to_bounds(&bounds, &map, x, y),
                    8.,
                    rgb(0xf0c36a),
                );
            }
            paint_dot(
                window,
                local_to_bounds(&bounds, &map, pose.0, pose.1),
                7.,
                rgb(0xf0b429),
            );
            if warn {
                paint_dot(
                    window,
                    local_to_bounds(&bounds, &map, pose.0, pose.1),
                    3.,
                    rgb(0xe24b4b),
                );
            }
        },
    )
    .absolute()
    .size_full()
}

fn local_to_bounds(
    bounds: &Bounds<Pixels>,
    map: &argos_core::LocalMap,
    x: f64,
    y: f64,
) -> Point<Pixels> {
    let fx = if map.span_x.abs() < 1e-6 {
        0.5
    } else {
        ((x - map.origin_x) / map.span_x).clamp(0., 1.)
    };
    let fy = if map.span_y.abs() < 1e-6 {
        0.5
    } else {
        ((y - map.origin_y) / map.span_y).clamp(0., 1.)
    };
    point(
        bounds.origin.x + bounds.size.width * fx as f32,
        bounds.origin.y + bounds.size.height * (1. - fy as f32),
    )
}

fn local_pixel(
    width: f32,
    height: f32,
    map: &argos_core::LocalMap,
    x: f64,
    y: f64,
) -> Point<Pixels> {
    let bounds = Bounds {
        origin: point(px(0.), px(0.)),
        size: size(px(width), px(height)),
    };
    local_to_bounds(&bounds, map, x, y)
}

fn point_to_local(
    frame: &Bounds<Pixels>,
    map: &argos_core::LocalMap,
    position: Point<Pixels>,
) -> Option<(f64, f64)> {
    let width = f32::from(frame.size.width);
    let height = f32::from(frame.size.height);
    if width <= 0. || height <= 0. {
        return None;
    }
    let fx = (f32::from(position.x - frame.origin.x) / width).clamp(0., 1.) as f64;
    let fy = (f32::from(position.y - frame.origin.y) / height).clamp(0., 1.) as f64;
    Some((
        map.origin_x + fx * map.span_x,
        map.origin_y + (1. - fy) * map.span_y,
    ))
}

fn paint_dot(window: &mut Window, at: Point<Pixels>, span: f32, color: Rgba) {
    window.paint_quad(quad(
        Bounds {
            origin: point(at.x - px(span * 0.5), at.y - px(span * 0.5)),
            size: size(px(span), px(span)),
        },
        Corners::default(),
        color,
        Edges::default(),
        transparent_black(),
        BorderStyle::default(),
    ));
}

fn stroke(window: &mut Window, from: Point<Pixels>, to: Point<Pixels>, color: Rgba, dashed: bool) {
    let dx = f32::from(to.x - from.x);
    let dy = f32::from(to.y - from.y);
    let len = dx.hypot(dy);
    if len < 1. {
        return;
    }
    let steps = (len / 5.).max(1.) as i32;
    for step in 0..steps {
        if dashed && step % 2 == 0 {
            continue;
        }
        let t = step as f32 / steps as f32;
        let at = point(from.x + px(dx * t), from.y + px(dy * t));
        window.paint_quad(quad(
            Bounds {
                origin: point(at.x - px(1.5), at.y - px(1.5)),
                size: size(px(3.), px(3.)),
            },
            Corners::default(),
            color,
            Edges::default(),
            transparent_black(),
            BorderStyle::default(),
        ));
    }
    if dashed {
        let at = point(from.x + px(dx * 0.72), from.y + px(dy * 0.72));
        window.paint_quad(quad(
            Bounds {
                origin: point(at.x - px(4.), at.y - px(4.)),
                size: size(px(8.), px(8.)),
            },
            Corners::default(),
            color,
            Edges::default(),
            transparent_black(),
            BorderStyle::default(),
        ));
    }
}

fn drag_handle_at(
    id: u64,
    anchor: DragAnchor,
    at: Point<Pixels>,
    on_map: bool,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let slot = match anchor {
        DragAnchor::Rover => 0,
        DragAnchor::Goal => 1,
    } + if on_map { 2 } else { 0 };
    div()
        .id(("drag", id.saturating_mul(4).saturating_add(slot)))
        .absolute()
        .left(at.x - px(9.))
        .top(at.y - px(9.))
        .w(px(18.))
        .h(px(18.))
        .rounded_md()
        .cursor_pointer()
        .bg(rgb(if anchor == DragAnchor::Goal {
            0x3dcc9a
        } else {
            0xf0b429
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.begin_drag(id, anchor, on_map);
                cx.stop_propagation();
                cx.notify();
            }),
        )
}

fn map_paint(dashboard: &Dashboard) -> impl IntoElement {
    let view = dashboard.view.clone();
    let drag = dashboard.drag.as_ref().map(|active| active.goal.clone());
    let marks = dashboard.marks.clone();
    let origin = dashboard.feed.origin();
    let show_occupancy = dashboard.show_occupancy;
    canvas(
        |bounds, _, _| bounds,
        move |_bounds, _, window, cx| {
            let map = view.read(cx);
            if show_occupancy {
                paint_occupancy(window, map, &marks, &origin);
            }
            for mark in &marks {
                let mut previous: Option<Point<Pixels>> = None;
                for sample in &mark.route {
                    let Ok((latitude, longitude)) = origin.geographic(sample.x, sample.y) else {
                        previous = None;
                        continue;
                    };
                    let Some(at) = map.project(latitude, longitude) else {
                        previous = None;
                        continue;
                    };
                    if let Some(from) = previous {
                        stroke(
                            window,
                            from,
                            at,
                            if sample.travel == Travel::Reverse {
                                rgb(0xf0c36a)
                            } else {
                                rgb(0x7eb6ff)
                            },
                            sample.travel == Travel::Reverse,
                        );
                    }
                    previous = Some(at);
                }
            }
            let Some(goal) = &drag else {
                return;
            };
            let Some(mark) = marks.iter().find(|mark| mark.id == goal.id) else {
                return;
            };
            let cursor = match goal.space {
                DragSpace::Geographic => Some((goal.cursor.x, goal.cursor.y)),
                DragSpace::Local => origin.geographic(goal.cursor.x, goal.cursor.y).ok(),
            };
            let Some((latitude, longitude)) = cursor else {
                return;
            };
            let Some(ghost) = map.project(latitude, longitude) else {
                return;
            };
            let rover_at = map.project(mark.latitude, mark.longitude);
            if let Some(rover) = rover_at {
                stroke(window, rover, ghost, rgb(0xf0c36a), false);
            }
            if let Some(radius) = goal.arrival_radius_m {
                let mut metres = ground_metres_per_pixel(map, latitude, longitude);
                if let Some(rover) = rover_at {
                    let span = f32::from(ghost.x - rover.x).hypot(f32::from(ghost.y - rover.y));
                    let distance = goal.distance_m();
                    if distance > 2. && span > 8. {
                        metres = distance / f64::from(span);
                    }
                }
                let pixels = (radius / metres).clamp(8., 240.) as f32;
                for step in 0..24 {
                    let angle = step as f32 / 24. * std::f32::consts::TAU;
                    let at = point(
                        ghost.x + px(pixels * angle.cos()),
                        ghost.y + px(pixels * angle.sin()),
                    );
                    window.paint_quad(quad(
                        Bounds {
                            origin: point(at.x - px(2.), at.y - px(2.)),
                            size: size(px(4.), px(4.)),
                        },
                        Corners::default(),
                        rgb(0xf0c36a),
                        Edges::default(),
                        transparent_black(),
                        BorderStyle::default(),
                    ));
                }
            }
            window.paint_quad(quad(
                Bounds {
                    origin: point(ghost.x - px(6.), ghost.y - px(6.)),
                    size: size(px(12.), px(12.)),
                },
                Corners::default(),
                rgb(0xf0c36a),
                Edges::default(),
                transparent_black(),
                BorderStyle::default(),
            ));
        },
    )
    .absolute()
    .size_full()
}

/// Local occupancy draped through the localization origin. Cells stay in the
/// rover frame; the overlay does not claim a geographic obstacle location.
fn paint_occupancy(
    window: &mut Window,
    map: &MapView,
    marks: &[RoverMark],
    origin: &argos_core::LocalFrame,
) {
    for mark in marks {
        let Some(grid) = &mark.grid else {
            continue;
        };
        if grid.cols == 0 {
            continue;
        }
        let metres = ground_metres_per_pixel(map, mark.latitude, mark.longitude).max(0.05);
        let cell_m = grid.span_x / f64::from(grid.cols);
        let pixels = (cell_m / metres).clamp(2., 40.) as f32;
        for (index, &(x, y)) in grid.centers.iter().enumerate() {
            let Some(class) = grid.cells.get(index).copied() else {
                continue;
            };
            if class == CellClass::Unknown {
                continue;
            }
            let Ok((latitude, longitude)) = origin.geographic(x, y) else {
                continue;
            };
            let Some(at) = map.project(latitude, longitude) else {
                continue;
            };
            let mut color = rgb(cell_color(class));
            color.a = match class {
                CellClass::Free => 0.38,
                CellClass::Occupied => 0.78,
                CellClass::Unknown => 0.,
            };
            window.paint_quad(quad(
                Bounds {
                    origin: point(at.x - px(pixels * 0.5), at.y - px(pixels * 0.5)),
                    size: size(px(pixels), px(pixels)),
                },
                Corners::default(),
                color,
                Edges::default(),
                transparent_black(),
                BorderStyle::default(),
            ));
        }
    }
}

/// Ground metres per pixel from the camera, measured near a coordinate.
/// The 3D view does not publish a ground scale, so two projected points supply it.
fn ground_metres_per_pixel(map: &MapView, latitude: f64, longitude: f64) -> f64 {
    let fallback = map.metres_per_pixel().unwrap_or(1.).max(0.05);
    let Some(origin) = map.project(latitude, longitude) else {
        return fallback;
    };
    // A long step can leave the 3D frustum and make project() return nothing.
    for step in [0.00002_f64, -0.00002, 0.00005] {
        let Some(other) = map.project(latitude + step, longitude) else {
            continue;
        };
        let pixels = f32::from(other.x - origin.x).hypot(f32::from(other.y - origin.y));
        let metres = geodesic_metres(latitude, longitude, latitude + step, longitude);
        if metres.is_finite() && metres > 0.2 && pixels > 2. {
            return (metres / f64::from(pixels)).max(0.02);
        }
    }
    fallback
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
    let config = DashboardConfig::from_env().unwrap_or_else(|error| {
        eprintln!("argos-gpui: {error}");
        std::process::exit(1);
    });
    // Resolve the tiles key before MapConfig reads the process environment.
    let maps_api_key = config.maps_api_key.clone();
    let maps_notice = config.maps_notice.clone();
    let feed = FleetFeed::start(config.clone());
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
                    title: Some("ARGOS".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(12.), px(12.))),
                }),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| Dashboard::new(map, feed, maps_notice, cx));
                let focus = view.read(cx).focus.clone();
                focus.focus(window, cx);
                view
            },
        )
        .expect("Failed to open window");
        cx.activate(true);
    });
}
