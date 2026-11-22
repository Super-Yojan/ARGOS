// Uses gilrs to read a game pad, and sends teleop commands to the rovers.
//
// Bindings (Xbox names; gilrs maps other pads to the same layout, and the UI
// shows PlayStation names — L1 / R1 / ○ / Options — for a PS pad):
//   LB (hold)       dead-man: no twist is sent unless it is held
//   Left stick Y    forward / back  → `Teleop.v`
//   Right stick X   turn            → `Teleop.w` (stick left = CCW +)
//   RB (hold)       turbo: full speed instead of the normal cap
//   D-pad ←/→       pick the previous / next rover
//   B               engage e-stop on the selected rover
//   Start           release this base's e-stop on the selected rover
//
// Safety: `BaseLink::send(Teleop)` is fire-and-forget and the rover keeps the
// last twist for `COMMAND_LEASE`, so on dead-man release (or pad disconnect,
// or rover switch) we send zero twists for a short while before going quiet.
// The loop runs on the GPUI foreground executor, so a frozen UI stops sending
// and the rover's lease brings it to a halt.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use terra_comms::{BaseLink, CommsConfig, Estop, TaskStatus, Teleop as TeleopCmd};

/// `BaseLink::send` docs: hold a stick by sending at 20 Hz.
const TICK: Duration = Duration::from_millis(50);
/// Zero twists sent after the dead-man is released (~0.5 s at 20 Hz). The
/// topic uses `Drop` QoS, so a single stop could be lost.
const STOP_TICKS: u32 = 10;
/// Stick values below this read as zero.
const DEADZONE: f32 = 0.12;

/// Speed caps, overridable with `TELEOP_MAX_V` / `TELEOP_MAX_W` / `TELEOP_TURBO_V` / `TELEOP_TURBO_W`.
#[derive(Clone, Copy)]
struct Limits {
    /// m/s
    max_v: f64,
    /// rad/s
    max_w: f64,
    turbo_v: f64,
    turbo_w: f64,
}

impl Limits {
    fn from_env() -> Self {
        let get = |key: &str, default: f64| {
            std::env::var(key)
                .ok()
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(default)
        };
        let max_v = get("TELEOP_MAX_V", 0.4);
        let max_w = get("TELEOP_MAX_W", 1.0);
        Self {
            max_v,
            max_w,
            turbo_v: get("TELEOP_TURBO_V", max_v * 2.0),
            turbo_w: get("TELEOP_TURBO_W", max_w * 1.5),
        }
    }
}

pub struct Teleop {
    gilrs: Option<Gilrs>,
    gilrs_error: Option<String>,
    /// The pad we read from: the last one that pressed a button.
    pad: Option<GamepadId>,

    endpoint: String,
    base: Option<BaseLink>,
    connect_error: Option<String>,

    rovers: Vec<u64>,
    selected: Option<u64>,
    /// Rovers this base has an engaged e-stop on.
    estopped: BTreeSet<u64>,

    limits: Limits,
    deadman: bool,
    turbo: bool,
    /// Last commanded twist (after scaling), for the UI.
    v: f64,
    w: f64,
    /// Zero twists still to send to `stop_target` after the dead-man let go.
    stop_ticks: u32,
    stop_target: Option<u64>,
    sent: u64,

    _poll_task: Task<()>,
}

impl Teleop {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (gilrs, gilrs_error) = match Gilrs::new() {
            Ok(g) => (Some(g), None),
            Err(gilrs::Error::NotImplemented(g)) => {
                (Some(g), Some("Gamepads aren't supported on this platform".into()))
            }
            Err(err) => (None, Some(err.to_string())),
        };
        let pad = gilrs
            .as_ref()
            .and_then(|g| g.gamepads().next().map(|(id, _)| id));

        let (endpoint, base, connect_error) = connect_from_env();

        let poll_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });

        Self {
            gilrs,
            gilrs_error,
            pad,
            endpoint,
            base,
            connect_error,
            rovers: Vec::new(),
            selected: None,
            estopped: BTreeSet::new(),
            limits: Limits::from_env(),
            deadman: false,
            turbo: false,
            v: 0.0,
            w: 0.0,
            stop_ticks: 0,
            stop_target: None,
            sent: 0,
            _poll_task: poll_task,
        }
    }

    /// One 20 Hz step: read the pad, update the rover list, send a twist.
    fn tick(&mut self, cx: &mut Context<Self>) {
        let before = self.snapshot();

        self.refresh_rovers();
        self.handle_pad_events();
        self.read_sticks();
        self.send_twist();

        // The rover list and stick values change all the time; only redraw
        // when something visible did.
        if self.snapshot() != before {
            cx.notify();
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            pad: self.pad,
            rovers: self.rovers.clone(),
            selected: self.selected,
            estopped: self.estopped.len(),
            deadman: self.deadman,
            turbo: self.turbo,
            v: (self.v * 100.0).round() as i64,
            w: (self.w * 100.0).round() as i64,
            connected: self.base.as_ref().is_some_and(BaseLink::connected),
        }
    }

    fn refresh_rovers(&mut self) {
        let Some(base) = &self.base else { return };
        self.rovers = base.rovers();
        // Auto-pick only when there is no ambiguity. A selected rover that
        // drops out stays selected, so it reappears without re-picking.
        if self.selected.is_none() && self.rovers.len() == 1 {
            self.selected = Some(self.rovers[0]);
        }
    }

    fn handle_pad_events(&mut self) {
        let mut actions = Vec::new();
        {
            let Some(gilrs) = &mut self.gilrs else { return };
            while let Some(ev) = gilrs.next_event() {
                match ev.event {
                    EventType::Connected => {
                        if self.pad.is_none() {
                            self.pad = Some(ev.id);
                        }
                    }
                    EventType::Disconnected => {
                        if self.pad == Some(ev.id) {
                            self.pad = gilrs.gamepads().next().map(|(id, _)| id);
                        }
                    }
                    EventType::ButtonPressed(button, _) => {
                        // Whichever pad is used becomes the active one.
                        self.pad = Some(ev.id);
                        actions.push(button);
                    }
                    _ => {}
                }
            }
        }
        for button in actions {
            match button {
                Button::DPadLeft => self.cycle_rover(-1),
                Button::DPadRight => self.cycle_rover(1),
                Button::East => self.set_estop(true),
                Button::Start => self.set_estop(false),
                _ => {}
            }
        }
    }

    fn read_sticks(&mut self) {
        let pad = self
            .gilrs
            .as_ref()
            .zip(self.pad)
            .and_then(|(g, id)| g.connected_gamepad(id));
        let Some(pad) = pad else {
            self.deadman = false;
            self.turbo = false;
            self.v = 0.0;
            self.w = 0.0;
            return;
        };

        self.deadman = pad.is_pressed(Button::LeftTrigger);
        self.turbo = pad.is_pressed(Button::RightTrigger);
        let (max_v, max_w) = if self.turbo {
            (self.limits.turbo_v, self.limits.turbo_w)
        } else {
            (self.limits.max_v, self.limits.max_w)
        };
        // gilrs: stick up = +Y, stick right = +X. `w` is CCW +, so negate X.
        self.v = deadzone(pad.value(Axis::LeftStickY)) as f64 * max_v;
        self.w = -deadzone(pad.value(Axis::RightStickX)) as f64 * max_w;
    }

    fn send_twist(&mut self) {
        let Some(base) = &self.base else { return };
        let target = self.selected;
        let driving = self.deadman
            && target.is_some_and(|id| !self.estopped.contains(&id));

        if driving {
            let id = target.unwrap();
            // Switched rover mid-drive: stop the old one first.
            if let Some(old) = self.stop_target.filter(|old| *old != id) {
                base.send(old, &zero_twist());
            }
            base.send(id, &TeleopCmd { t: now(), v: self.v, w: self.w });
            self.sent += 1;
            self.stop_target = Some(id);
            self.stop_ticks = STOP_TICKS;
        } else if self.stop_ticks > 0 {
            if let Some(id) = self.stop_target {
                base.send(id, &zero_twist());
                self.sent += 1;
            }
            self.stop_ticks -= 1;
            if self.stop_ticks == 0 {
                self.stop_target = None;
            }
        }
    }

    fn cycle_rover(&mut self, step: isize) {
        if self.rovers.is_empty() {
            return;
        }
        let n = self.rovers.len() as isize;
        let next = match self.selected.and_then(|id| self.rovers.iter().position(|r| *r == id)) {
            Some(ix) => (ix as isize + step).rem_euclid(n),
            None if step < 0 => n - 1,
            None => 0,
        };
        self.select(self.rovers[next as usize]);
    }

    fn select(&mut self, id: u64) {
        if self.selected != Some(id) {
            self.selected = Some(id);
            // `send_twist` sends the old rover zeros while stop_ticks lasts.
        }
    }

    fn set_estop(&mut self, engaged: bool) {
        let (Some(base), Some(id)) = (&self.base, self.selected) else { return };
        base.send(id, &Estop { t: now(), engaged });
        if engaged {
            self.estopped.insert(id);
        } else {
            self.estopped.remove(&id);
        }
    }
}

#[derive(PartialEq)]
struct Snapshot {
    pad: Option<GamepadId>,
    rovers: Vec<u64>,
    selected: Option<u64>,
    estopped: usize,
    deadman: bool,
    turbo: bool,
    v: i64,
    w: i64,
    connected: bool,
}

/// Same env vars as the map view: `TERRA_ENDPOINT`, `TERRA_PREFIX`,
/// `TERRA_USER`, `TERRA_PASSWORD`.
fn connect_from_env() -> (String, Option<BaseLink>, Option<String>) {
    let endpoint =
        std::env::var("TERRA_ENDPOINT").unwrap_or_else(|_| "tcp/127.0.0.1:7447".into());
    let prefix = std::env::var("TERRA_PREFIX").unwrap_or_else(|_| "terra".into());
    let user = std::env::var("TERRA_USER").ok().filter(|s| !s.is_empty());
    let password = std::env::var("TERRA_PASSWORD").ok().filter(|s| !s.is_empty());

    let mut toml = format!("[comms]\nendpoint = \"{endpoint}\"\nprefix = \"{prefix}\"\n");
    if let Some(u) = &user {
        toml.push_str(&format!("user = \"{u}\"\n"));
    }

    match CommsConfig::from_toml(&toml)
        .map(|cfg| cfg.with_password(password))
        .and_then(|cfg| terra_comms::connect(&cfg))
    {
        Ok(link) => (endpoint, Some(link.base()), None),
        Err(err) => (endpoint, None, Some(err.to_string())),
    }
}

/// Zero inside the dead zone, then rescaled so the output still spans 0..=1.
fn deadzone(x: f32) -> f32 {
    let mag = x.abs();
    if !mag.is_finite() || mag < DEADZONE {
        return 0.0;
    }
    (x.signum() * (mag - DEADZONE) / (1.0 - DEADZONE)).clamp(-1.0, 1.0)
}

fn zero_twist() -> TeleopCmd {
    TeleopCmd { t: now(), v: 0.0, w: 0.0 }
}

/// Sender clock, seconds (`terra-comms`: every message carries `t`).
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_sm()
        .border_1()
        .border_color(cx.theme().border)
        .cursor_pointer()
        .text_color(cx.theme().foreground)
        .child(label.into())
}

/// A centred bar: fills left for negative, right for positive.
fn gauge(label: &str, value: f64, max: f64, unit: &str, cx: &App) -> Div {
    let frac = if max > 0.0 { (value / max).clamp(-1.0, 1.0) } else { 0.0 } as f32;
    let half = || div().flex_1().h_full().relative();
    let fill = || div().absolute().top_0().h_full().bg(cx.theme().primary);
    v_flex()
        .w(px(260.0))
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .text_xs()
                .child(div().text_color(cx.theme().muted_foreground).child(label.to_string()))
                .child(
                    div()
                        .text_color(cx.theme().foreground)
                        .child(format!("{value:+.2} {unit}")),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .h(px(10.0))
                .rounded_sm()
                .overflow_hidden()
                .bg(cx.theme().muted)
                .child(half().when(frac < 0.0, |h| {
                    h.child(fill().right_0().w(relative(-frac)))
                }))
                .child(div().w(px(1.0)).h_full().bg(cx.theme().border))
                .child(half().when(frac > 0.0, |h| h.child(fill().left_0().w(relative(frac))))),
        )
}

impl Render for Teleop {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = self.base.as_ref().is_some_and(BaseLink::connected);
        let link_status = if let Some(err) = &self.connect_error {
            format!("Config error: {err}")
        } else if connected {
            format!("Connected · {}", self.endpoint)
        } else {
            format!("Connecting to {}…", self.endpoint)
        };

        let pad_name = self
            .gilrs
            .as_ref()
            .zip(self.pad)
            .and_then(|(g, id)| g.connected_gamepad(id))
            .map(|p| p.name().to_string());
        let pad_status = match (&pad_name, &self.gilrs_error) {
            (Some(name), _) => format!("Gamepad: {name}"),
            (None, Some(err)) => format!("Gamepad error: {err}"),
            (None, None) => "No gamepad connected".to_string(),
        };

        let labels = ButtonLabels::for_pad(pad_name.as_deref());

        // terra-av drives from the teleop twist only while it has no goal
        // (`pipeline.rs`: `Goal::None` + teleop → velocity). A running task
        // with another controller would win over the stick.
        let rover_info = self.selected.and_then(|id| {
            self.base
                .as_ref()
                .and_then(|b| b.latest::<TaskStatus>(id))
                .map(|(s, _)| (s.task_id, s.controller, s.state))
        });
        let estopped_here = self.selected.is_some_and(|id| self.estopped.contains(&id));
        let estopped_remote = self.selected.is_some_and(|id| {
            self.base
                .as_ref()
                .and_then(|b| b.latest::<terra_comms::Status>(id))
                .is_some_and(|(s, _)| s.estopped)
        });

        let drive_state = if self.selected.is_none() {
            ("Pick a rover", cx.theme().muted_foreground)
        } else if estopped_here || estopped_remote {
            ("E-STOPPED", cx.theme().danger)
        } else if self.deadman && pad_name.is_some() {
            ("DRIVING", rgb(0x22c55e).into())
        } else {
            (labels.hold_to_drive, cx.theme().muted_foreground)
        };

        let dot = |ok: bool| {
            div()
                .w(px(8.0))
                .h(px(8.0))
                .rounded_full()
                .bg(if ok { rgb(0x22c55e) } else { rgb(0xf59e0b) })
        };

        let header = v_flex()
            .px_3()
            .py_2()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_xs()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(dot(connected))
                    .child(div().text_color(cx.theme().foreground).child(link_status)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(dot(pad_name.is_some()))
                    .child(div().text_color(cx.theme().foreground).child(pad_status)),
            );

        let rover_buttons = h_flex().gap_2().flex_wrap().children(
            self.rovers.iter().map(|&id| {
                let active = self.selected == Some(id);
                button(SharedString::from(format!("rover-{id}")), format!("Rover {id}"), cx)
                    .when(active, |b| {
                        b.bg(cx.theme().primary)
                            .text_color(cx.theme().primary_foreground)
                            .border_color(cx.theme().primary)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(id);
                        cx.notify();
                    }))
            }),
        );

        let rovers = v_flex()
            .gap_2()
            .child(div().text_sm().text_color(cx.theme().foreground).child("Rovers"))
            .when(self.rovers.is_empty(), |c| {
                c.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No rovers seen in the last few seconds"),
                )
            })
            .child(rover_buttons)
            .when_some(rover_info, |c, (task_id, controller, state)| {
                let (text, warn) = match task_id {
                    None => ("No task · stick drives the rover".to_string(), false),
                    Some(id) if controller == "teleop" => {
                        (format!("Task {id} · teleop · {state:?}"), false)
                    }
                    Some(id) => (
                        format!("Task {id} · {controller} · {state:?} — the task overrides the stick"),
                        true,
                    ),
                };
                c.child(
                    div()
                        .text_xs()
                        .text_color(if warn {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(text),
                )
            });

        let drive = v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .text_color(drive_state.1)
                            .child(drive_state.0),
                    )
                    .when(self.turbo && self.deadman, |r| {
                        r.child(div().text_xs().text_color(cx.theme().danger).child("TURBO"))
                    }),
            )
            .child(gauge(
                "Linear (v)",
                self.v,
                self.limits.turbo_v,
                "m/s",
                cx,
            ))
            .child(gauge(
                "Angular (w)",
                self.w,
                self.limits.turbo_w,
                "rad/s",
                cx,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Commands sent: {}", self.sent)),
            );

        let estop = h_flex()
            .gap_2()
            .child(
                div()
                    .id("estop-engage")
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(cx.theme().danger)
                    .text_color(cx.theme().danger_foreground)
                    .font_weight(FontWeight::BOLD)
                    .cursor_pointer()
                    .child("E-STOP")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_estop(true);
                        cx.notify();
                    })),
            )
            .child(
                button("estop-release", "Release", cx)
                    .py_2()
                    .when(!estopped_here, |b| b.opacity(0.5))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_estop(false);
                        cx.notify();
                    })),
            )
            .when(estopped_remote && !estopped_here, |r| {
                r.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child("E-stop engaged elsewhere (phone or another base)"),
                )
            });

        let l = self.limits;
        let help = div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(format!(
                "{} hold: dead-man · Left stick: drive (≤ {:.2} m/s) · Right stick: turn (≤ {:.2} rad/s) · \
                 {}: turbo · D-pad ←/→: rover · {}: e-stop · {}: release",
                labels.deadman, l.max_v, l.max_w, labels.turbo, labels.estop, labels.release
            ));

        v_flex().size_full().child(header).child(
            v_flex()
                .p_4()
                .gap_5()
                .child(rovers)
                .child(drive)
                .child(estop)
                .child(help),
        )
    }
}

/// Button names as printed on the connected pad. gilrs reports every pad in
/// the Xbox layout, so only the labels change.
struct ButtonLabels {
    deadman: &'static str,
    turbo: &'static str,
    estop: &'static str,
    release: &'static str,
    hold_to_drive: &'static str,
}

impl ButtonLabels {
    fn for_pad(name: Option<&str>) -> Self {
        let name = name.unwrap_or_default().to_ascii_lowercase();
        let playstation = ["ps3", "ps4", "ps5", "dualshock", "dualsense", "sony", "playstation"]
            .iter()
            .any(|k| name.contains(k));
        if playstation {
            Self {
                deadman: "L1",
                turbo: "R1",
                estop: "○",
                release: "Options",
                hold_to_drive: "Hold L1 to drive",
            }
        } else {
            Self {
                deadman: "LB",
                turbo: "RB",
                estop: "B",
                release: "Start",
                hold_to_drive: "Hold LB to drive",
            }
        }
    }
}

impl Drop for Teleop {
    fn drop(&mut self) {
        // Don't leave a rover coasting on its last command for COMMAND_LEASE.
        if let (Some(base), Some(id)) = (&self.base, self.stop_target) {
            base.send(id, &zero_twist());
        }
    }
}
