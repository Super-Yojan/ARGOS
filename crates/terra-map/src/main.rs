use gpui_kit::*;
use terra_map::{Map, MapConfig, MapView};

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(640.), px(480.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Terra · spatial explorer".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(12.), px(12.))),
                }),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| {
                    MapView::new(
                        Map::new(MapConfig::default()).expect("Invalid map config"),
                        cx,
                    )
                });
                view.read(cx).focus_handle(cx).focus(window, cx);
                view
            },
        )
        .expect("Failed to open window");
        cx.activate(true);
    });
}
