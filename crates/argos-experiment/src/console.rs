//! The console's shell: the top bar and whichever page is open.
//!
//! It owns the navigation between pages and nothing else. It also redraws
//! the window a few times a second, because rover data arrives on the
//! link's own threads and nothing else tells the UI about it.

use std::time::Duration;

use gpui_kit::component::v_flex;
use gpui_kit::*;

use crate::components::chat::AgentChat;
use crate::components::fpv_view::FpvView;
use crate::components::scene_view::SceneView;
use crate::components::top_bar::TopBar;
use crate::components::widgets::{ButtonLook, button, segment, segmented};
use crate::fleet::{Fleet, Level};
use crate::pages::drive::DrivePage;
use crate::pages::supervise::{SuperviseEvent, SupervisePage};
use crate::theme;

/// How often the console looks at the rovers' latest messages.
const REFRESH_PERIOD: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Supervise,
    Drive,
    Agent,
}

pub struct Console {
    fleet: Fleet,
    view: View,
    scene: Entity<SceneView>,
    _fpv: Entity<FpvView>,
    supervise: Entity<SupervisePage>,
    drive: Entity<DrivePage>,
    agent: Entity<AgentChat>,
    _refresh_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Console {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fleet = Fleet::connect_from_env();

        // One 3D view and one FPV view, shared by whichever page is open.
        let scene = cx.new(|cx| SceneView::new(&fleet, cx));
        let fpv = cx.new(|cx| FpvView::new(&fleet, cx));
        let supervise =
            cx.new(|cx| SupervisePage::new(fleet.clone(), scene.clone(), fpv.clone(), cx));
        let drive =
            cx.new(|cx| DrivePage::new(fleet.clone(), scene.clone(), fpv.clone(), window, cx));
        let agent = cx.new(|cx| AgentChat::new(window, cx));

        let subscriptions = vec![cx.subscribe_in(
            &supervise,
            window,
            |this, _, event, window, cx| match event {
                SuperviseEvent::TakeOver { rover, level } => {
                    this.take_over(*rover, *level, window, cx)
                }
            },
        )];

        let refresh_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH_PERIOD).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break; // The window is gone.
                }
            }
        });

        Self {
            fleet,
            view: View::Supervise,
            scene,
            _fpv: fpv,
            supervise,
            drive,
            agent,
            _refresh_task: refresh_task,
            _subscriptions: subscriptions,
        }
    }

    fn take_over(
        &mut self,
        rover_id: u64,
        level: Level,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drive
            .update(cx, |drive, cx| drive.take_over(rover_id, level, window, cx));
        self.view = View::Drive;
        cx.notify();
    }

    fn release_drive(&mut self, cx: &mut Context<Self>) {
        self.drive.update(cx, |drive, cx| drive.release(cx));
        self.view = View::Supervise;
        cx.notify();
    }

    fn show(&mut self, view: View, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }

    fn save_snapshot(&mut self, cx: &mut Context<Self>) {
        self.scene.update(cx, |scene, cx| scene.save_snapshot(cx));
    }

    fn top_bar(&self, cx: &mut Context<Self>) -> TopBar {
        let bar = TopBar::new(self.fleet.clone());
        let save_button = button("top-save-snapshot", "Save snapshot", ButtonLook::Outline)
            .on_click(cx.listener(|this, _, _, cx| this.save_snapshot(cx)));

        if self.view == View::Drive {
            let drive = self.drive.read(cx);
            return bar
                .driving(drive.banner(cx))
                .child(save_button)
                .child(
                    button("release-drive", drive.release_label(), ButtonLook::Outline)
                        .border_color(rgb(theme::BLUE))
                        .text_color(rgb(theme::BLUE_PALE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .on_click(cx.listener(|this, _, _, cx| this.release_drive(cx))),
                );
        }

        let tab = |id: &'static str, label: &'static str, view: View| {
            segment(id, label, self.view == view)
                .on_click(cx.listener(move |this, _, _, cx| this.show(view, cx)))
        };
        bar.child(save_button).child(
            segmented()
                .child(tab("view-supervise", "Supervise", View::Supervise))
                .child(tab("view-agent", "Agent", View::Agent)),
        )
    }
}

impl Render for Console {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.view {
            View::Supervise => self.supervise.clone().into_any_element(),
            View::Drive => self.drive.clone().into_any_element(),
            View::Agent => div()
                .size_full()
                .rounded(px(10.0))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .bg(rgb(theme::PANEL))
                .overflow_hidden()
                .child(self.agent.clone())
                .into_any_element(),
        };

        v_flex()
            .size_full()
            .p_3()
            .gap_3()
            .bg(rgb(theme::BG))
            .text_color(rgb(theme::TEXT))
            .font_family(theme::FONT_UI)
            .text_size(px(14.0))
            .child(self.top_bar(cx))
            .child(div().flex_1().min_h_0().child(page))
    }
}
