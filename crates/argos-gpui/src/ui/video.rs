//! Video tab: the rover's FPV picture, call state, and stats.

use super::widgets::hint;
use crate::Dashboard;

use argos_gpui::FpvPhase;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{ActiveTheme, Sizable, button::Button};
use gpui_kit::{prelude::*, *};

/// Video stays with the selection: Start FPV asks the selected rover's phone.
/// Controls sit on top and the picture takes the rest of the panel, so a
/// short dock panel never hides Start FPV below the fold.
pub(super) fn video_tab(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let picture = dashboard.fpv.view();
    let phase = picture.phase;
    let detail = picture.detail.clone();
    let turns = picture
        .phone_id
        .or(dashboard.selected)
        .and_then(|id| picture.rotations.get(&id).copied())
        .unwrap_or(0);
    let empty = empty_picture_text(dashboard.selected, &picture);
    v_flex()
        .size_full()
        .p_4()
        .gap_2()
        .bg(cx.theme().background)
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("fpv-start")
                        .outline()
                        .small()
                        .label("Start FPV")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(id) = this.selected {
                                this.fpv.start_phone(id);
                                this.record_fpv_action("start", Some(id), None);
                                this.note = format!("FPV requested for rover {id}");
                            } else {
                                this.note = "Select a rover before starting FPV".into();
                            }
                            cx.notify();
                        })),
                )
                .child(stop_fpv_button(cx))
                .child(
                    Button::new("fpv-rotate")
                        .outline()
                        .small()
                        .label(format!("Rotate {}°", u32::from(turns) * 90))
                        .tooltip("Turn the picture 90° clockwise, for a phone mounted sideways")
                        .on_click(cx.listener(|this, _, _, cx| {
                            let target = this.fpv.view().phone_id.or(this.selected);
                            if let Some(id) = target {
                                let turns = this.fpv.rotate(id);
                                this.record_fpv_action("rotate", Some(id), Some(turns));
                                this.note =
                                    format!("Rover {id} video turned {}°", u32::from(turns) * 90);
                            } else {
                                this.note = "Select a rover before rotating video".into();
                            }
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(phase_color(phase, cx))
                        .child(format!("{} · {detail}", phase.label())),
                ),
        )
        .child(hint(fpv_subject(dashboard.selected, picture.phone_id), cx))
        .child(
            div()
                .w_full()
                .flex_1()
                .min_h(px(120.))
                .bg(cx.theme().secondary)
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
                            .child(empty),
                    )
                }),
        )
        .child(hint(fpv_stats(&picture), cx))
}

/// What the empty picture says: the next step, not a stale default.
fn empty_picture_text(selected: Option<u64>, picture: &argos_gpui::FpvView) -> String {
    match (picture.phase, selected) {
        (FpvPhase::Idle | FpvPhase::Closed, Some(id)) => {
            format!("Press Start FPV to watch rover {id}")
        }
        (FpvPhase::Idle | FpvPhase::Closed, None) => "Select a rover, then press Start FPV".into(),
        _ => picture.detail.clone(),
    }
}

pub(super) fn stop_fpv_button(cx: &mut Context<Dashboard>) -> Button {
    Button::new("fpv-stop")
        .outline()
        .small()
        .label("Stop FPV")
        .on_click(cx.listener(|this, _, _, cx| {
            let watching = this.fpv.view().phone_id;
            this.fpv.stop();
            this.record_fpv_action("stop", watching, None);
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
