//! ARGOS operator dashboard.
//!
//! The camera starts at `ARGOS_MAP_START_LAT` / `ARGOS_MAP_START_LON` until rover
//! positions arrive, then recenters on the fleet. `MAPS_DEMO=1` still draws the
//! offline fixture. Clicking a rover selects it and takes local control.
//! With a rover selected, clicking the map proposes a destination there and
//! opens a small popup at the click; Send goal publishes it. Esc, Cancel, or a
//! point on the rover sends nothing. An acknowledged outdoor GPS rover uses the
//! same click: the ghost shows the arrival radius and GPS distance, and Send
//! goal publishes the WGS84 point as the explicit operator action.
//!
//! This file holds the window's state (`Dashboard`) and `main`. What the
//! window draws and what its buttons do are in `ui/`; see `ui/mod.rs` for a
//! file-by-file list.

mod ui;

use ui::{agent, palette};

use argos_core::{ControlClaim, GoalDrag};
use argos_gpui::console::{InspectorTab, StopOrigin};
use argos_gpui::{DashboardConfig, FleetFeed, FpvController, LaunchMode, RoverMark, RoverPresence};
use gpui_kit::component::{Theme, ThemeMode, TitleBar, input::InputState};
use gpui_kit::{prelude::*, *};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use terra_map::{Map, MapConfig, MapMode, MapView, ObjectId};

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
    framed_center: Option<(f64, f64)>,
    marks: Vec<RoverMark>,
    presence: Vec<RoverPresence>,
    selected: Option<u64>,
    /// A selected rover that dropped out of the fleet keeps its map focus
    /// and is selected again when it returns.
    selection_hold: argos_gpui::SelectionHold,
    map_mode: MapMode,
    drag: Option<ActiveDrag>,
    /// Where the left button went down on the map, to tell a click from an
    /// orbit or pan.
    press: Option<Point<Pixels>>,
    /// The map area and the inspector, measured at prepaint for rover card
    /// placement.
    field_frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
    inspector_frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
    show_occupancy: bool,
    inspector_tab: InspectorTab,
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
    /// Stops this window sent and not yet reset: control and window clock.
    latched: BTreeMap<u64, (StopOrigin, f64)>,
    /// Rovers last seen reporting `emergency_stop`, with their reason, so each
    /// new rover-side stop or change of reason is logged once.
    rover_stops: BTreeMap<u64, Option<String>>,
    /// Index into `EXPLORE_BUDGETS_MIN` for the next L4 Explore request.
    explore_budget: usize,
    /// Typed L5 task, and the reply box for the model's questions.
    task_input: Entity<InputState>,
    task_reply: Entity<InputState>,
    /// Index into `TASK_BUDGETS_MIN` for the next L5 task.
    task_budget: usize,
    /// Answerer picked for the next L5 task; `None` prefers the phone's MCP agent.
    task_agent: Option<argos_core::TaskAgent>,
    fpv: FpvController,
    /// Study condition chosen in this window: `None` until the experimenter
    /// picks one, then `Some(None)` for off or `Some(Some(level))` for a fixed
    /// level. Until then the window never sends one, so a restarted window
    /// does not clear a running trial's condition.
    study: Option<Option<&'static str>>,
    /// Per rover: when the condition was last sent and how often since it changed.
    study_sends: BTreeMap<u64, (f64, u32)>,
    /// Which fleet `feed` and `fpv` are connected to. The console toggle
    /// replaces both.
    fleet_mode: LaunchMode,
    /// Agent bridge, mode, recommendation cards, and raw-drive grant.
    agent: agent::AgentDesk,
    _poll: Task<()>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = argos_gpui::agent_mcp::agent_cli(&args) {
        std::process::exit(code);
    }
    let mode = match LaunchMode::parse(args) {
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
                    title: Some(mode.title().into()),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
            },
            cx,
            |window, cx| {
                // Kit widgets follow the light theme on the console background.
                // The status colors are darkened so they read as text too.
                Theme::change(ThemeMode::Light, Some(window), cx);
                Theme::update(cx, |theme| {
                    theme.background = rgb(palette::BACKGROUND).into();
                    theme.title_bar = rgb(palette::BACKGROUND).into();
                    theme.foreground = rgb(palette::TEXT).into();
                    theme.muted_foreground = rgb(palette::MUTED).into();
                    let status = |color: u32| {
                        let color: Hsla = rgb(color).into();
                        (color, color.opacity(0.9), color.opacity(0.8))
                    };
                    (theme.warning, theme.warning_hover, theme.warning_active) =
                        status(palette::WARNING);
                    (theme.success, theme.success_hover, theme.success_active) =
                        status(palette::SUCCESS);
                    (theme.danger, theme.danger_hover, theme.danger_active) =
                        status(palette::DANGER);
                    theme.warning_foreground = gpui::white();
                });
                let view =
                    cx.new(|cx| Dashboard::new(map, feed, fpv, maps_notice, mode, window, cx));
                let focus = view.read(cx).focus.clone();
                focus.focus(window, cx);
                view
            },
        )
        .expect("Failed to open window");
        cx.activate(true);
    });
}
