//! ARGOS operator dashboard.
//!
//! Opens one window on the Map View and subscribes, read-only, to a Terra or
//! Zorvane Zenoh session. The camera opens at `ARGOS_MAP_START_LAT` /
//! `ARGOS_MAP_START_LON` until rover positions arrive, then recenters on the
//! fleet. `MAPS_DEMO=1` still draws the offline fixture when Zenoh is down.
//! `GOOGLE_MAPS_API_KEY` loads Photorealistic 3D Tiles. No API key is stored.

use argos_gpui::{DashboardConfig, FleetFeed, framing_target};
use gpui_kit::{prelude::*, *};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;
use terra_map::{Color, GeoPosition, Map, MapConfig, MapEvent, MapView, ObjectId};

const MARKER_ALTITUDE: f64 = 80.;
const MARKER_SCALE: f64 = 12.;

struct Dashboard {
    map: Map,
    view: Entity<MapView>,
    feed: FleetFeed,
    model: PathBuf,
    objects: BTreeMap<u64, ObjectId>,
    framed: BTreeSet<u64>,
    rows: Vec<(u64, String, bool)>,
    link: String,
    detail: String,
    _poll: Task<()>,
}
impl Dashboard {
    fn new(map: Map, feed: FleetFeed, cx: &mut Context<Self>) -> Self {
        let model =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../terra-map/src/assets/robot.glb");
        let view = cx.new(|cx| MapView::new(map.clone(), cx));
        let poll = cx.spawn(async |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        while let Some(event) = this.map.poll_event() {
                            if let MapEvent::ObjectLoadFailed { message, .. } = event {
                                this.detail = message;
                            }
                        }
                        this.sync();
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
            model,
            objects: BTreeMap::new(),
            framed: BTreeSet::new(),
            rows: Vec::new(),
            link: initial.link,
            detail: initial.detail,
            _poll: poll,
        }
    }
    fn sync(&mut self) {
        let picture = self.feed.picture();
        self.link.clone_from(&picture.link);
        self.detail.clone_from(&picture.detail);
        self.rows = picture
            .marks
            .iter()
            .map(|mark| {
                (
                    mark.id,
                    format!("Rover {} · {} · {}", mark.id, mark.status, mark.occupancy),
                    mark.status != "online",
                )
            })
            .collect();
        let seen: BTreeSet<u64> = picture.marks.iter().map(|mark| mark.id).collect();
        let gone: Vec<u64> = self
            .objects
            .keys()
            .copied()
            .filter(|id| !seen.contains(id))
            .collect();
        for id in gone {
            if let Some(object) = self.objects.remove(&id) {
                let _ = self.map.remove_object(object);
            }
        }
        for mark in &picture.marks {
            let position = GeoPosition::new(mark.latitude, mark.longitude, MARKER_ALTITUDE);
            let object = if let Some(object) = self.objects.get(&mark.id).copied() {
                if self.map.set_object_position(object, position).is_err() {
                    continue;
                }
                object
            } else {
                match self.map.add_3d_obj(position, &self.model) {
                    Ok(object) => {
                        let _ = self.map.set_object_scale(object, MARKER_SCALE);
                        self.objects.insert(mark.id, object);
                        object
                    }
                    Err(error) => {
                        self.detail = error.to_string();
                        continue;
                    }
                }
            };
            let _ = self.map.set_object_heading(object, mark.heading);
            if mark.status == "online" {
                let _ = self.map.clear_illumination(object);
            } else {
                let _ = self
                    .map
                    .illuminate_3d_object(object, Color::rgb(255, 160, 50));
            }
        }
        if seen.is_empty() {
            self.framed.clear();
        } else if let Some((ids, latitude, longitude)) =
            framing_target(&self.framed, &picture.marks)
            && self.map.show_lat_lon(latitude, longitude).is_ok()
        {
            self.framed = ids;
        }
    }
}
impl Render for Dashboard {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let empty = self.rows.is_empty();
        div()
            .size_full()
            .flex()
            .bg(rgb(0x0b141d))
            .text_color(rgb(0xd8e7ec))
            .font_family(".SystemUIFont")
            .child(
                div()
                    .w(px(280.))
                    .h_full()
                    .flex_none()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_size(px(23.)).child("ARGOS"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(0x98aeb9))
                            .child(format!("{} · {}", self.link, self.detail)),
                    )
                    .children(self.rows.iter().map(|(id, row, attention)| {
                        div()
                            .id(("rover", *id))
                            .p_3()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if *attention { 0xc47b2a } else { 0x263d48 }))
                            .bg(rgb(0x14232e))
                            .text_size(px(13.))
                            .child(row.clone())
                    }))
                    .when(empty, |this| {
                        this.child(
                            div().text_size(px(12.)).text_color(rgb(0x98aeb9)).child(
                                "No rover position yet. The map stays at the start location.",
                            ),
                        )
                    })
                    .child(
                        div().mt_3().text_size(px(11.)).text_color(rgb(0x98aeb9)).child(
                            "Read-only. Drag to orbit, Shift-drag to pan, scroll to zoom. Markers are 12× scale at 80 m ellipsoid height.",
                        ),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.view.clone()),
            )
    }
}
fn main() {
    let config = DashboardConfig::from_env().unwrap_or_else(|error| {
        eprintln!("argos-gpui: {error}");
        std::process::exit(1);
    });
    let feed = FleetFeed::start(config.clone());
    let map = Map::new(MapConfig {
        latitude: config.start_latitude,
        longitude: config.start_longitude,
        show_controls: false,
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
                let view = cx.new(|cx| Dashboard::new(map, feed, cx));
                view.read(cx)
                    .view
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                view
            },
        )
        .expect("Failed to open window");
        cx.activate(true);
    });
}
