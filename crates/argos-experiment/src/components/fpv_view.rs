//! Live WebRTC FPV video view for a rover's phone camera.
//!
//! Renders decoded H.264 frames from [`FpvController`] as a GPUI [`RenderImage`],
//! with a compact status/metrics badge and Start / Stop / Rotate controls.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::widgets::mono;
use crate::fleet::Fleet;
use crate::fpv::{FpvConfig, FpvController, FpvPhase, FpvView as FpvSnapshot};
use crate::theme;

/// How often the view checks [`FpvController`] for a new decoded frame.
const POLL_PERIOD: Duration = Duration::from_millis(33);

pub struct FpvView {
    fleet: Fleet,
    controller: FpvController,
    target_rover: Option<u64>,
    snapshot: FpvSnapshot,
    last_key: (u64, FpvPhase),
    /// Currently displayed frame; kept so we can drop the previous GPU texture.
    displayed: Option<Arc<RenderImage>>,
    _poll_task: Task<()>,
}

impl FpvView {
    pub fn new(fleet: &Fleet, cx: &mut Context<Self>) -> Self {
        let controller = FpvController::start(fleet.base_arc(), FpvConfig::from_env());
        let snapshot = controller.view();
        let last_key = controller.snapshot_key();

        let poll_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_PERIOD).await;
                let alive = this.update(cx, |this, cx| {
                    let key = this.controller.snapshot_key();
                    if key != this.last_key {
                        this.last_key = key;
                        this.snapshot = this.controller.view();
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        Self {
            fleet: fleet.clone(),
            controller,
            target_rover: None,
            snapshot,
            last_key,
            displayed: None,
            _poll_task: poll_task,
        }
    }

    /// Update which rover this view targets. If an FPV stream is already active
    /// for a different rover, switches the stream to the new rover.
    pub fn set_target_rover(&mut self, rover_id: Option<u64>, cx: &mut Context<Self>) {
        if self.target_rover == rover_id {
            return;
        }
        self.target_rover = rover_id;
        if let Some(id) = rover_id
            && self.is_active()
            && self.snapshot.phone_id != Some(id)
        {
            self.start_for(id, cx);
            return;
        }
        cx.notify();
    }

    /// Start (or restart) an FPV session for `rover_id`.
    pub fn start_for(&mut self, rover_id: u64, cx: &mut Context<Self>) {
        self.target_rover = Some(rover_id);
        self.controller.start_phone(rover_id);
        self.snapshot = self.controller.view();
        self.last_key = self.controller.snapshot_key();
        cx.notify();
    }

    /// Stop the current FPV session.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.controller.stop();
        self.snapshot = self.controller.view();
        self.last_key = self.controller.snapshot_key();
        cx.notify();
    }

    /// Rotate the displayed video by +90 degrees clockwise.
    pub fn rotate(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.resolved_rover() {
            self.controller.rotate(id);
            self.snapshot = self.controller.view();
            self.last_key = self.controller.snapshot_key();
            cx.notify();
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.snapshot.phase,
            FpvPhase::Starting | FpvPhase::Connecting | FpvPhase::Connected
        )
    }

    fn resolved_rover(&self) -> Option<u64> {
        self.target_rover
            .or(self.snapshot.phone_id)
            .or_else(|| self.fleet.rovers().first().map(|r| r.id))
    }
}

impl Render for FpvView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Drop any frames retired by the decoder so GPUI frees their textures.
        for retired in self.controller.take_retired() {
            let _ = window.drop_image(retired);
        }
        if let Some(next_frame) = &self.snapshot.image {
            let is_new = self
                .displayed
                .as_ref()
                .is_none_or(|cur| !Arc::ptr_eq(cur, next_frame));
            if is_new && let Some(old) = self.displayed.replace(next_frame.clone()) {
                let _ = window.drop_image(old);
            }
        } else if let Some(old) = self.displayed.take() {
            let _ = window.drop_image(old);
        }

        let rover_id = self.resolved_rover();
        let active = self.is_active();
        let phase_color = match self.snapshot.phase {
            FpvPhase::Connected => theme::GREEN,
            FpvPhase::Starting | FpvPhase::Connecting => theme::BLUE,
            FpvPhase::Failed => theme::RED.fill,
            FpvPhase::Idle | FpvPhase::Closed => theme::TEXT_MUTED,
        };

        let status = match self.snapshot.phase {
            FpvPhase::Connected => {
                let mut parts = vec![format!(
                    "FPV · R{} · {:.0} fps · {:.0} kbps",
                    self.snapshot.phone_id.unwrap_or(0),
                    self.snapshot.fps,
                    self.snapshot.bitrate_kbps
                )];
                if let Some(rtt) = self.snapshot.rtt_ms {
                    parts.push(format!("{rtt:.0} ms"));
                }
                if self.snapshot.width > 0 && self.snapshot.height > 0 {
                    parts.push(format!("{}×{}", self.snapshot.width, self.snapshot.height));
                }
                parts.join(" · ")
            }
            FpvPhase::Idle | FpvPhase::Closed => match rover_id {
                Some(id) => format!("FPV · Rover {id} · {}", self.snapshot.phase.label()),
                None => "FPV · no rover".into(),
            },
            phase => match self.snapshot.phone_id.or(rover_id) {
                Some(id) => format!("FPV · Rover {id} · {}", phase.label()),
                None => format!("FPV · {}", phase.label()),
            },
        };

        let overlay_button = |id: &'static str, label: &'static str| {
            mono(label)
                .id(id)
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .bg(rgb(theme::PANEL))
                .text_color(rgb(theme::TEXT_MUTED))
                .cursor_pointer()
                .occlude()
        };

        let placeholder = self.displayed.is_none().then(|| {
            let is_err = self.snapshot.phase == FpvPhase::Failed;
            let msg = if is_err || active {
                self.snapshot.detail.clone()
            } else if let Some(id) = rover_id {
                format!("Click Start FPV to stream Rover {id}'s camera")
            } else {
                "Waiting for a rover to connect…".into()
            };
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .px_4()
                .child(
                    mono(msg)
                        .text_size(px(12.0))
                        .text_color(rgb(if is_err {
                            theme::RED.accent
                        } else {
                            theme::TEXT_MUTED
                        })),
                )
        });

        div()
            .id("fpv-view")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(theme::SCENE_BG))
            .children(
                self.displayed
                    .clone()
                    .map(|frame| img(frame).size_full().object_fit(ObjectFit::Contain)),
            )
            .children(placeholder)
            .child(
                mono(status)
                    .absolute()
                    .left(px(10.0))
                    .top_2()
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .bg(rgb(theme::PANEL))
                    .text_color(rgb(phase_color)),
            )
            .child(
                h_flex()
                    .absolute()
                    .right_2()
                    .top_2()
                    .gap_1()
                    .when(active, |row| {
                        row.child(
                            overlay_button("fpv-rotate", "Rotate")
                                .on_click(cx.listener(|this, _, _, cx| this.rotate(cx))),
                        )
                        .child(
                            overlay_button("fpv-stop", "Stop FPV")
                                .text_color(rgb(theme::RED.accent))
                                .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                        )
                    })
                    .when(!active && rover_id.is_some(), |row| {
                        let id = rover_id.unwrap();
                        row.child(
                            overlay_button("fpv-start", "Start FPV")
                                .text_color(rgb(theme::GREEN))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.start_for(id, cx);
                                })),
                        )
                    }),
            )
    }
}
