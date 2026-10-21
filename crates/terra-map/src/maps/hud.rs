//! Dark maritime instrument UI, following the user's city-map reference.
use super::{MapView, camera::MapMode};
use gpui_kit::{img, prelude::*, *};

const CANVAS: u32 = 0x0b141d;
const PANEL: u32 = 0x101f2b;
const TEXT: u32 = 0xe5f1f5;
const MUTED: u32 = 0x9bb4c2;
const ACCENT: u32 = 0x86dce8;
const RULE: u32 = 0x2d4859;
const ACTIVE: u32 = 0x193847;

fn label(text: impl Into<SharedString>) -> Div {
    div()
        .font_family("Menlo")
        .text_size(px(11.))
        .text_color(rgb(MUTED))
        .child(text.into())
}
fn metric(name: &'static str, value: String, unit: &'static str) -> Div {
    div()
        .flex()
        .items_end()
        .justify_between()
        .py_3()
        .border_b_1()
        .border_color(rgb(RULE))
        .child(
            div().child(label(name)).child(
                div()
                    .mt_1()
                    .font_family("Menlo")
                    .text_size(px(22.))
                    .text_color(rgb(TEXT))
                    .child(value),
            ),
        )
        .child(label(unit))
}
fn control(
    id: &'static str,
    text: impl Into<SharedString>,
    active: bool,
    action: fn(&mut MapView, &mut Context<MapView>),
    cx: &mut Context<MapView>,
) -> Stateful<Div> {
    div()
        .id(id)
        .focusable()
        .tab_index(0)
        .occlude()
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .min_w(px(40.))
        .h(px(40.))
        .px_3()
        .rounded(px(4.))
        .border_1()
        .border_color(rgb(if active { ACCENT } else { RULE }))
        .bg(rgb(if active { ACTIVE } else { PANEL }))
        .text_color(rgb(if active { ACCENT } else { TEXT }))
        .text_size(px(13.))
        .hover(|s| s.bg(rgb(ACTIVE)).border_color(rgb(ACCENT)))
        .focus(|s| s.border_color(rgb(ACCENT)).bg(rgb(ACTIVE)))
        .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                action(this, cx);
                cx.stop_propagation();
            }
        }))
        .child(text.into())
}
fn compass(heading: f64) -> impl IntoElement {
    div()
        .relative()
        .w(px(96.))
        .h(px(96.))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let center = bounds.center();
                    let radius = 35.;
                    let mut ring = PathBuilder::stroke(px(1.));
                    for i in 0..=96 {
                        let a = i as f32 / 96. * std::f32::consts::TAU;
                        let p = point(
                            center.x + px(a.sin() * radius),
                            center.y - px(a.cos() * radius),
                        );
                        if i == 0 {
                            ring.move_to(p);
                        } else {
                            ring.line_to(p);
                        }
                    }
                    if let Ok(path) = ring.build() {
                        window.paint_path(path, rgb(RULE));
                    }
                    let mut ticks = PathBuilder::stroke(px(1.));
                    for i in 0..36 {
                        let a = i as f32 / 36. * std::f32::consts::TAU;
                        let length = if i % 9 == 0 { 7. } else { 3. };
                        ticks.move_to(point(
                            center.x + px(a.sin() * radius),
                            center.y - px(a.cos() * radius),
                        ));
                        ticks.line_to(point(
                            center.x + px(a.sin() * (radius - length)),
                            center.y - px(a.cos() * (radius - length)),
                        ));
                    }
                    if let Ok(path) = ticks.build() {
                        window.paint_path(path, rgb(MUTED));
                    }
                    let a = heading as f32;
                    let end = point(center.x + px(a.sin() * 23.), center.y - px(a.cos() * 23.));
                    let mut needle = PathBuilder::stroke(px(1.5));
                    needle.move_to(center);
                    needle.line_to(end);
                    needle.move_to(point(center.x - px(4.), center.y));
                    needle.line_to(point(center.x + px(4.), center.y));
                    needle.move_to(point(center.x, center.y - px(4.)));
                    needle.line_to(point(center.x, center.y + px(4.)));
                    if let Ok(path) = needle.build() {
                        window.paint_path(path, rgb(ACCENT));
                    }
                },
            )
            .size_full(),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(44.))
                .text_size(px(11.))
                .text_color(rgb(ACCENT))
                .child("N"),
        )
}

impl MapView {
    fn zoom_in(&mut self, cx: &mut Context<Self>) {
        self.flight = None;
        self.camera.zoom(120.);
        self.send();
        cx.notify();
    }
    fn zoom_out(&mut self, cx: &mut Context<Self>) {
        self.flight = None;
        self.camera.zoom(-120.);
        self.send();
        cx.notify();
    }
    fn survey(&mut self, cx: &mut Context<Self>) {
        if self.config.survey {
            return;
        }
        self.config.survey = true;
        self.epoch = self.epoch.wrapping_add(1);
        self.send();
        cx.notify();
    }
    fn imagery(&mut self, cx: &mut Context<Self>) {
        if !self.config.survey {
            return;
        }
        self.config.survey = false;
        self.epoch = self.epoch.wrapping_add(1);
        self.send();
        cx.notify();
    }
    fn two_d(&mut self, cx: &mut Context<Self>) {
        self.set_mode(MapMode::TwoD, cx);
    }
    fn three_d(&mut self, cx: &mut Context<Self>) {
        self.set_mode(MapMode::ThreeD, cx);
    }
    fn retry(&mut self, cx: &mut Context<Self>) {
        // Re-read a key that may have been configured before this process launched.
        if self.config.api_key.is_none() {
            self.config.api_key = std::env::var("GOOGLE_MAPS_API_KEY").ok();
        }
        self.worker = super::worker::MapWorker::start_with_map(
            self.config.clone(),
            Some(self.controller.clone()),
        );
        self.status = "Connecting to map tiles".into();
        self.send();
        cx.notify();
    }
    pub(super) fn render_map(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        for image in self.retired.drain(..) {
            let _ = window.drop_image(image);
        }
        let wide = f32::from(window.viewport_size().width) >= 900.
            && f32::from(window.viewport_size().height) >= 680.;
        let has_image = self.image.is_some();
        let weak = cx.entity().downgrade();
        let latitude = format!(
            "{:.5}° {}",
            self.camera.latitude.abs(),
            if self.camera.latitude >= 0. { "N" } else { "S" }
        );
        let longitude = format!(
            "{:.5}° {}",
            self.camera.longitude.abs(),
            if self.camera.longitude >= 0. {
                "E"
            } else {
                "W"
            }
        );
        let resolution = self
            .viewport
            .map(|(w, h)| format!("{w} × {h}"))
            .unwrap_or_else(|| "—".into());
        let heading = self.camera.heading.to_degrees().rem_euclid(360.);
        let map = div()
            .id("map-viewport")
            .absolute()
            .size_full()
            .overflow_hidden()
            .bg(rgb(CANVAS))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.flight = None;
                    this.drag = Some(event.position);
                    this.focus.focus(window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if let Some(last) =
                    previous_map_drag(&mut this.drag, event.position, event.dragging())
                {
                    let x = f32::from(event.position.x - last.x) as f64;
                    let y = f32::from(event.position.y - last.y) as f64;
                    if event.modifiers.shift || this.camera.mode == MapMode::TwoD {
                        this.camera.pan_in_view(
                            x,
                            y,
                            f32::from(window.viewport_size().height) as f64,
                        );
                    } else {
                        this.camera.orbit(x, y);
                    }
                    this.send();
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.drag = None),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                this.flight = None;
                this.camera
                    .zoom(f32::from(event.delta.pixel_delta(px(24.)).y) as f64);
                this.send();
                cx.notify();
                cx.stop_propagation();
            }))
            .when_some(self.image.clone(), |el, image| {
                el.child(
                    img(image)
                        .absolute()
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                )
            })
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let _ =
                            weak.update(cx, |this, _| this.resize(bounds, window.scale_factor()));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        let sidebar = div()
            .occlude()
            .absolute()
            .top(px(100.))
            .left_6()
            .w(px(232.))
            .max_h(window.viewport_size().height - px(208.))
            .id("map-instruments")
            .overflow_y_scroll()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(RULE))
            .bg(rgba(0x101f2bec))
            .p_4()
            .child(label("VIEW PARAMETERS"))
            .child(metric(
                "CAMERA RANGE",
                format!("{:.2}", self.camera.distance / 1000.),
                "km",
            ))
            .child(metric(
                "VIEW MODE",
                if self.camera.mode == MapMode::TwoD {
                    "2D".into()
                } else {
                    "3D".into()
                },
                "",
            ))
            .child(metric(
                "PITCH",
                format!("{:.0}", self.camera.pitch.to_degrees()),
                "deg",
            ))
            .child(div().mt_6().child(label("RENDERING")))
            .child(
                div()
                    .mt_3()
                    .flex()
                    .justify_between()
                    .child(label("Visible tiles"))
                    .child(self.tile_count.to_string()),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .justify_between()
                    .child(label("Frame size"))
                    .child(resolution.clone()),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .justify_between()
                    .child(label("Detail error"))
                    .child(format!("{:.0} px", self.config.screen_space_error)),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .justify_between()
                    .child(label("Frame / GPU copy"))
                    .child(format!(
                        "{:.1} / {:.1} ms",
                        self.metrics.render_ms, self.metrics.gpu_wait_ms
                    )),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .justify_between()
                    .child(label("Tile download"))
                    .child(format!("{:.0} ms", self.metrics.download_ms)),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .justify_between()
                    .child(label("Decode / upload"))
                    .child(format!(
                        "{:.0} / {:.1} ms",
                        self.metrics.decode_ms, self.metrics.upload_ms
                    )),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .justify_between()
                    .child(label("Pending / cached"))
                    .child(format!(
                        "{} / {}",
                        self.metrics.pending, self.metrics.resident
                    )),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .justify_between()
                    .child(label("Poll / image prep"))
                    .child(format!(
                        "{:.1} / {:.1} ms",
                        self.metrics.delivery_ms, self.metrics.ui_image_ms
                    )),
            )
            .child(div().mt_6().child(label("NAVIGATION")))
            .child(
                div()
                    .mt_3()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(if self.camera.mode == MapMode::TwoD {
                        "Drag to pan. Scroll to zoom. Arrow keys to move. Press 3 for 3D."
                    } else {
                        "Drag to orbit. Shift-drag to pan. Scroll to zoom. Press 2 for 2D."
                    }),
            )
            .child(
                div()
                    .mt_4()
                    .child(control("recenter", "Recenter · R", false, Self::reset, cx)),
            );
        let coordinate = div()
            .occlude()
            .absolute()
            .left_6()
            .bottom(px(108.))
            .font_family("Menlo")
            .text_size(px(12.))
            .text_color(rgb(TEXT))
            .bg(rgba(0x101f2bec))
            .rounded(px(4.))
            .px_3()
            .py_2()
            .child(format!("{latitude}   {longitude}"));
        let navigation = div()
            .occlude()
            .absolute()
            .right_6()
            .top(px(100.))
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .child(compass(self.camera.heading))
            .child(
                div()
                    .font_family("Menlo")
                    .text_size(px(13.))
                    .child(format!("{heading:03.0}°")),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(control(
                        "mode-2d",
                        "2D",
                        self.camera.mode == MapMode::TwoD,
                        Self::two_d,
                        cx,
                    ))
                    .child(control(
                        "mode-3d",
                        "3D",
                        self.camera.mode == MapMode::ThreeD,
                        Self::three_d,
                        cx,
                    )),
            )
            .child(control("zoom-in", "+", false, Self::zoom_in, cx))
            .child(control("zoom-out", "−", false, Self::zoom_out, cx))
            .when(!wide, |el| {
                el.child(control("reset-camera", "Reset", false, Self::reset, cx))
            });
        let layer_bar = div()
            .occlude()
            .absolute()
            .bottom(px(106.))
            .right_6()
            .flex()
            .gap_2()
            .child(control(
                "survey-layer",
                "Survey",
                self.config.survey,
                Self::survey,
                cx,
            ))
            .child(control(
                "imagery-layer",
                "Imagery",
                !self.config.survey,
                Self::imagery,
                cx,
            ));
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(CANVAS))
            .text_color(rgb(TEXT))
            .text_size(px(14.))
            .font_family(".SystemUIFont")
            .track_focus(&self.focus)
            .tab_index(0)
            .on_key_down(cx.listener(Self::key))
            .child(map)
            .when(self.config.show_controls, |el| {
                el.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(76.))
                        .occlude()
                        .bg(rgba(0x0b141def))
                        .border_b_1()
                        .border_color(rgb(RULE))
                        .px_6()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_4()
                                .child(
                                    div()
                                        .text_size(px(23.))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(ACCENT))
                                        .child("ARGOS"),
                                )
                                .child(div().w(px(1.)).h(px(24.)).bg(rgb(RULE)))
                                .child(div().child(div().text_size(px(14.)).child("Map")).child(
                                    label(if self.config.demo {
                                        "OFFLINE GPU FIXTURE"
                                    } else {
                                        "GOOGLE PHOTOREALISTIC 3D"
                                    }),
                                )),
                        )
                        .child(control(
                            "reload-tiles",
                            "Reload tiles",
                            false,
                            Self::retry,
                            cx,
                        )),
                )
            })
            .when(wide && self.config.show_controls, |el| el.child(sidebar))
            .when(self.config.show_controls, |el| el.child(navigation))
            .when(has_image && self.config.show_controls, |el| {
                el.child(coordinate).child(layer_bar)
            })
            .when(!has_image, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(150.))
                        .left(px(if wide { 300. } else { 24. }))
                        .right(px(145.))
                        .bottom(px(140.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .max_w(px(500.))
                                .occlude()
                                .p_6()
                                .bg(rgba(0x101f2bec))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(rgb(RULE))
                                .child(label("SCENE STATUS"))
                                .child(div().mt_4().text_size(px(20.)).child(if self.config.demo {
                                    "Preparing the GPU fixture"
                                } else {
                                    "Connect the map"
                                }))
                                .child(
                                    div()
                                        .mt_3()
                                        .text_color(rgb(MUTED))
                                        .child(self.status.clone()),
                                )
                                .child(div().mt_6().flex().gap_2().child(control(
                                    "retry-tiles",
                                    "Retry connection",
                                    false,
                                    Self::retry,
                                    cx,
                                ))),
                        ),
                )
            })
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .min_h(px(80.))
                    .occlude()
                    .bg(rgb(PANEL))
                    .border_t_1()
                    .border_color(rgb(RULE))
                    .px_6()
                    .py_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .child(if self.config.demo {
                                        "Offline GPU fixture"
                                    } else {
                                        "Google Maps"
                                    }),
                            )
                            .child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(
                                if self.status.is_empty() {
                                    format!("{} tiles in view · {resolution}", self.tile_count)
                                } else {
                                    self.status.clone()
                                },
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(self.credits.clone()),
                    ),
            )
    }
}

// Movement may continue a map press, but must never acquire another element's drag.
fn previous_map_drag(
    last: &mut Option<Point<Pixels>>,
    next: Point<Pixels>,
    pressed: bool,
) -> Option<Point<Pixels>> {
    if !pressed {
        *last = None;
        return None;
    }
    let previous = (*last)?;
    *last = Some(next);
    Some(previous)
}

#[cfg(test)]
#[path = "../../../../tests/terra-map/unit/maps_hud_drag_tests.rs"]
mod drag_tests;
