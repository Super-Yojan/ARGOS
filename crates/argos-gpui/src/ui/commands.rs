//! The app menu bar and its keyboard shortcuts. Every toolbar command is
//! also here, so it can be found and used from the keyboard. Space (stop the
//! selected rover) and W/A/S/D (drive) stay raw key handlers in `render.rs`
//! because they must work while held and while a button has focus.

use super::panels::PanelKind;
use crate::Dashboard;

use argos_gpui::LaunchMode;
use argos_gpui::console::StopOrigin;
use gpui_kit::{prelude::*, *};
use terra_map::MapMode;

actions!(
    argos,
    [
        Quit,
        StopAll,
        StopRover,
        ResetStop,
        TakeOver,
        ControlFleet,
        ExportLog,
        UseSimulator,
        UseRealRovers,
        Map2D,
        Map3D,
        ToggleGrid,
        ToggleMuted,
        ToggleFollow,
        ShowDetails,
        ShowStudy,
        ShowGoal,
        ShowMission,
        ShowVideo,
        ShowAgent,
        ShowTimeline,
        TalkToAgents,
    ]
);

/// Key bindings and the menu bar. `secondary` is Cmd on macOS, Ctrl elsewhere.
pub(crate) fn install(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-.", StopAll, None),
        KeyBinding::new("secondary-r", ResetStop, None),
        KeyBinding::new("secondary-t", TakeOver, None),
        KeyBinding::new("secondary-shift-c", ControlFleet, None),
        KeyBinding::new("secondary-e", ExportLog, None),
        KeyBinding::new("secondary-2", Map2D, None),
        KeyBinding::new("secondary-3", Map3D, None),
        KeyBinding::new("secondary-g", ToggleGrid, None),
        KeyBinding::new("secondary-shift-m", ToggleMuted, None),
        KeyBinding::new("secondary-shift-f", ToggleFollow, None),
        KeyBinding::new("alt-secondary-1", ShowDetails, None),
        KeyBinding::new("alt-secondary-2", ShowStudy, None),
        KeyBinding::new("alt-secondary-3", ShowGoal, None),
        KeyBinding::new("alt-secondary-4", ShowMission, None),
        KeyBinding::new("alt-secondary-5", ShowVideo, None),
        KeyBinding::new("alt-secondary-6", ShowAgent, None),
        KeyBinding::new("alt-secondary-7", ShowTimeline, None),
        KeyBinding::new("secondary-shift-v", TalkToAgents, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.set_menus([
        Menu::new("ARGOS").items([MenuItem::action("Quit ARGOS", Quit)]),
        Menu::new("View").items([
            MenuItem::action("2D Map", Map2D),
            MenuItem::action("3D Map", Map3D),
            MenuItem::action("Follow Selected Rover", ToggleFollow),
            MenuItem::separator(),
            MenuItem::action("Muted Imagery", ToggleMuted),
            MenuItem::action("Occupancy Grid", ToggleGrid),
            MenuItem::separator(),
            MenuItem::action(PanelKind::Details.title(), ShowDetails),
            MenuItem::action(PanelKind::Autonomy.title(), ShowStudy),
            MenuItem::action(PanelKind::Goal.title(), ShowGoal),
            MenuItem::action(PanelKind::Mission.title(), ShowMission),
            MenuItem::action(PanelKind::Video.title(), ShowVideo),
            MenuItem::action(PanelKind::Agent.title(), ShowAgent),
            MenuItem::action(PanelKind::Timeline.title(), ShowTimeline),
        ]),
        Menu::new("Rover").items([
            MenuItem::action("Take Over (L0 Manual)", TakeOver),
            MenuItem::action("Stop Selected Rover (Space)", StopRover),
            MenuItem::action("Reset Stop", ResetStop),
        ]),
        Menu::new("Agent").items([
            MenuItem::action("Talk to Agents (start / stop)", TalkToAgents),
            MenuItem::action(PanelKind::Agent.title(), ShowAgent),
        ]),
        Menu::new("Fleet").items([
            MenuItem::action("Control Fleet / Release", ControlFleet),
            MenuItem::action("Stop All Rovers", StopAll),
            MenuItem::separator(),
            MenuItem::action("Use Simulator", UseSimulator),
            MenuItem::action("Use Real Rovers", UseRealRovers),
            MenuItem::separator(),
            MenuItem::action("Export Session Log", ExportLog),
        ]),
    ]);
}

impl Dashboard {
    /// Take or release this window's control lease. Releasing also ends
    /// driving and clears the selection.
    pub(crate) fn toggle_claim(&mut self) {
        if self.claim.held(self.now) {
            self.claim.release();
            self.stop_drive();
            self.drag = None;
            self.selected = None;
            self.selection_hold.clear();
            self.note = "Fleet control released".into();
        } else {
            self.claim.take(self.now);
            self.note = "This window controls the fleet".into();
        }
    }
}

/// Wire each menu action to the dashboard. Called on the window's root div.
pub(crate) fn handlers(root: Stateful<Div>, cx: &mut Context<Dashboard>) -> Stateful<Div> {
    fn panel<A: Action>(
        root: Stateful<Div>,
        kind: PanelKind,
        cx: &mut Context<Dashboard>,
    ) -> Stateful<Div> {
        root.on_action(cx.listener(move |this, _: &A, window, cx| {
            this.docks.toggle(kind, window, cx);
            cx.notify();
        }))
    }
    let root = root
        .on_action(cx.listener(|this, _: &StopAll, _, cx| {
            this.stop_all(StopOrigin::StopAll);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &StopRover, _, cx| {
            this.estop();
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ResetStop, _, cx| {
            if let Some(id) = this.selected {
                this.reset_safety(id);
            }
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &TakeOver, window, cx| {
            if this.selected.is_some() {
                this.begin_drive(window, cx);
            } else {
                this.note = "Select a rover to take over".into();
            }
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ControlFleet, _, cx| {
            this.toggle_claim();
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ExportLog, _, cx| {
            this.export_session();
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &UseSimulator, window, cx| {
            this.switch_fleet(LaunchMode::Sim, window);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &UseRealRovers, window, cx| {
            this.switch_fleet(LaunchMode::Real, window);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &Map2D, _, cx| {
            this.set_map_mode(MapMode::TwoD);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &Map3D, _, cx| {
            this.set_map_mode(MapMode::ThreeD);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ToggleGrid, _, cx| {
            this.show_occupancy = !this.show_occupancy;
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &TalkToAgents, _, cx| {
            this.toggle_voice();
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ToggleFollow, _, cx| {
            this.toggle_follow();
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ToggleMuted, _, cx| {
            this.muted_map = !this.muted_map;
            this.map.set_survey(this.muted_map);
            cx.notify();
        }));
    let root = panel::<ShowDetails>(root, PanelKind::Details, cx);
    let root = panel::<ShowStudy>(root, PanelKind::Autonomy, cx);
    let root = panel::<ShowGoal>(root, PanelKind::Goal, cx);
    let root = panel::<ShowMission>(root, PanelKind::Mission, cx);
    let root = panel::<ShowVideo>(root, PanelKind::Video, cx);
    let root = panel::<ShowAgent>(root, PanelKind::Agent, cx);
    panel::<ShowTimeline>(root, PanelKind::Timeline, cx)
}
