//! Run: cargo run --example fleet -- [path/to/robot.glb]
//! MAPS_DEMO=1 uses a synthetic offline map. No fleet types are required by the crate.
use gpui_kit::{prelude::*, *};
use std::{path::PathBuf, time::Duration};
use terra_map::{Color, GeoPosition, Map, MapConfig, MapEvent, MapMode, MapView, ObjectId};

use std::sync::Arc;

struct Robot {
    name: &'static str,
    location: GeoPosition,
    object: ObjectId,
    needs_attention: bool,
}
struct FleetView {
    map: Map,
    view: Entity<MapView>,
    robots: Vec<Robot>,
    selected: Option<usize>,
    status: String,
    _poll: Task<()>,
}
impl FleetView {
    fn new(cx: &mut Context<Self>) -> Self {
        let map = Map::new(MapConfig {
            show_controls: false,
            ..Default::default()
        })
        .unwrap();
        let model = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/assets/robot.glb")
            });
        let robots = [
            ("Rover 01", 37.7749, -122.4194, false),
            ("Rover 02", 37.7760, -122.4180, true),
            ("Rover 03", 37.7738, -122.4210, false),
        ]
        .into_iter()
        .map(|(name, lat, lon, attention)| {
            let location = GeoPosition::new(lat, lon, 80.);
            let object = map.add_3d_obj(location, &model).unwrap();
            // Exaggerated for visibility at neighborhood scale; glTF units are meters.
            map.set_object_scale(object, 12.).unwrap();
            if attention {
                map.illuminate_3d_object(object, Color::rgb(255, 160, 50))
                    .unwrap();
            }
            Robot {
                name,
                location,
                object,
                needs_attention: attention,
            }
        })
        .collect();
        let view = cx.new(|cx| MapView::new(map.clone(), cx));
        let poll = cx.spawn(async |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        while let Some(event) = this.map.poll_event() {
                            this.status = match event {
                                MapEvent::ObjectLoaded { .. } => {
                                    "Model ready · select a robot to recenter".into()
                                }
                                MapEvent::ObjectLoadFailed { message, .. } => message,
                            };
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            map,
            view,
            robots,
            selected: None,
            status: "Loading local robot model…".into(),
            _poll: poll,
        }
    }
    fn robot_clicked(&mut self, index: usize, cx: &mut Context<Self>) {
        let location = self.robots[index].location;
        // A normal Rust fleet callback can hold map.clone() and make this same call.
        self.map
            .show_lat_lon(location.latitude, location.longitude)
            .unwrap();
        self.selected = Some(index);
        cx.notify();
    }
}
impl Render for FleetView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().bg(rgb(0x0b141d)).text_color(rgb(0xd8e7ec)).font_family(".SystemUIFont")
            .child(div().w(px(260.)).h_full().flex_none().p_5().flex().flex_col().gap_3()
                .child(div().text_size(px(23.)).child("terra / fleet"))
                .child(div().text_size(px(12.)).text_color(rgb(0x98aeb9)).child("Select a robot to move the map"))
                .children(self.robots.iter().enumerate().map(|(index, robot)| {
                    div().id(("robot", index)).p_3().rounded_md().cursor_pointer().border_1()
                        .border_color(rgb(if self.selected == Some(index) { 0x93d4ce } else { 0x263d48 }))
                        .bg(rgb(0x14232e))
                        .child(robot.name)
                        .child(div().mt_1().text_size(px(12.)).text_color(rgb(0x98aeb9)).child(if robot.needs_attention { "Needs attention" } else { "Operating normally" }))
                        .on_click(cx.listener(move |this, _, _, cx| this.robot_clicked(index, cx)))
                }))
                .child(div().id("toggle-attention").p_3().border_1().border_color(rgb(0x263d48)).rounded_md().cursor_pointer()
                    .child("Toggle selected alert")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(index) = this.selected {
                            let robot = &mut this.robots[index]; robot.needs_attention = !robot.needs_attention;
                            if robot.needs_attention { this.map.illuminate_3d_object(robot.object, Color::rgb(255, 160, 50)).unwrap(); }
                            else { this.map.clear_illumination(robot.object).unwrap(); }
                            cx.notify();
                        }
                    })))
                .child(div().flex().gap_2().children([(MapMode::TwoD, "2D"), (MapMode::ThreeD, "3D")].into_iter().map(|(mode, title)| {
                    div().id(title).p_3().cursor_pointer().bg(rgb(0x20333f)).rounded_md().child(title)
                        .on_click(cx.listener(move |this, _, _, _| this.map.set_mode(mode)))
                })))
                .child(div().mt_3().text_size(px(12.)).text_color(rgb(0x98aeb9)).child(self.status.clone()))
                .child(div().text_size(px(11.)).text_color(rgb(0x98aeb9)).child("Models shown at 12× scale, 80 m ellipsoid height. Drag to orbit; Shift-drag to pan; scroll to zoom.")))
            .child(div().relative().flex_1().min_w_0().h_full().child(self.view.clone()))
    }
}
fn main() {
    gpui_kit::application().run(|cx: &mut App| {
        gpui_kit::init(cx);
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.), px(480.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Terra · fleet example".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(FleetView::new),
        )
        .unwrap();
        cx.activate(true);
    });
}
