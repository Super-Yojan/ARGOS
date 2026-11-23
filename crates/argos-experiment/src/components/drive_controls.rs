//! Drive one rover by hand, from the keyboard or a gamepad.
//!
//! This component owns the input state and the loop that sends the stick
//! to the rover 20 times a second. It draws the right-hand column of the
//! drive view: the state of the drive link, the obstacle assist, the speed
//! and the stop button.
//!
//! Two levels of autonomy live here (see `fleet::level`):
//!
//! - **L0 Manual**: the stick goes to the rover as it is.
//! - **L1 Assisted**: the rover reports the obstacles it sees, and the stick
//!   is slowed or stopped in front of them (`fleet::assist`) before it is
//!   sent.
//!
//! Keyboard (while the drive view has focus):
//!   W / S or ↑ / ↓   forward / back
//!   A / D or ← / →   turn left / right
//!
//! Gamepad (Xbox names; a PlayStation pad shows its own):
//!   LB (hold)        enable the sticks
//!   Left stick Y     forward / back
//!   Right stick X    turn
//!   RB (hold)        turbo
//!   B / Start        engage / release the e-stop
//!
//! The rover drives from the stick only while no autonomous task is
//! running, and forgets a command after `terra_comms::COMMAND_LEASE`. So
//! "stop" is simply "stop sending": releasing the keys, losing focus or a
//! frozen window all bring the rover to a halt. A few zero commands are
//! still sent after a release so it stops at once instead of a lease later.

use std::time::Duration;

use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};
use gpui_kit::component::v_flex;
use gpui_kit::*;

use super::widgets::{ButtonLook, button, facts, heading, mono, note, panel, segment, segmented};
use crate::fleet::assist::{self, Verdict};
use crate::fleet::{Activity, Fleet, Level, LinkState, RoverAction};
use crate::theme;

/// The rate the link asks for while a stick is held.
const TICK: Duration = Duration::from_millis(50);
/// Zero commands sent after the stick is let go (half a second). The
/// teleop topic may drop messages, so one would not be enough.
const STOP_TICKS: u32 = 10;
/// Gamepad stick values below this read as zero.
const DEADZONE: f32 = 0.12;

/// Forward speed in m/s and turn rate in rad/s (counter-clockwise +).
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Twist {
    pub v: f64,
    pub w: f64,
}

/// The fastest the console asks the rover to go. Override with
/// `TELEOP_MAX_V`, `TELEOP_MAX_W`, `TELEOP_TURBO_V` and `TELEOP_TURBO_W`.
#[derive(Clone, Copy)]
struct Limits {
    normal: Twist,
    turbo: Twist,
}

impl Limits {
    fn from_env() -> Self {
        let read = |key: &str, default: f64| {
            std::env::var(key)
                .ok()
                .and_then(|text| text.parse::<f64>().ok())
                .filter(|value| value.is_finite() && *value > 0.0)
                .unwrap_or(default)
        };
        let normal = Twist {
            v: read("TELEOP_MAX_V", 0.4),
            w: read("TELEOP_MAX_W", 1.0),
        };
        let turbo = Twist {
            v: read("TELEOP_TURBO_V", normal.v * 2.0),
            w: read("TELEOP_TURBO_W", normal.w * 1.5),
        };
        Self { normal, turbo }
    }
}

/// Which drive keys are held down.
#[derive(Clone, Copy, Default, PartialEq)]
struct Keys {
    forward: bool,
    back: bool,
    left: bool,
    right: bool,
}

impl Keys {
    fn any(self) -> bool {
        self.forward || self.back || self.left || self.right
    }

    /// Set the key named `key` to `held`. False if it isn't a drive key.
    fn set(&mut self, key: &str, held: bool) -> bool {
        let slot = match key {
            "w" | "up" => &mut self.forward,
            "s" | "down" => &mut self.back,
            "a" | "left" => &mut self.left,
            "d" | "right" => &mut self.right,
            _ => return false,
        };
        *slot = held;
        true
    }
}

pub struct DriveControls {
    fleet: Fleet,
    /// The rover being driven; `None` while nobody is.
    rover: Option<u64>,

    /// Keys only arrive while this handle has focus.
    focus: FocusHandle,
    keys: Keys,
    turbo: bool,
    limits: Limits,

    /// The obstacle assist is on (level L1).
    assist: bool,
    /// What the assist did to the last command. `None`: it has no fresh
    /// obstacle report from the rover, so it holds the rover still.
    verdict: Option<Verdict>,

    gamepads: Option<Gilrs>,
    gamepad_error: Option<String>,
    /// The pad in use: the last one that had a button pressed.
    pad: Option<GamepadId>,

    /// The command sent on the last tick, for the display.
    twist: Twist,
    /// Zero commands still owed to `stopping` after the stick was let go.
    stop_ticks: u32,
    stopping: Option<u64>,

    _tick_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl DriveControls {
    pub fn new(fleet: Fleet, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (gamepads, gamepad_error) = match Gilrs::new() {
            Ok(gamepads) => (Some(gamepads), None),
            Err(gilrs::Error::NotImplemented(gamepads)) => (
                Some(gamepads),
                Some("not supported on this platform".to_string()),
            ),
            Err(error) => (None, Some(error.to_string())),
        };
        let pad = gamepads
            .as_ref()
            .and_then(|gamepads| gamepads.gamepads().next().map(|(id, _)| id));

        let focus = cx.focus_handle();
        // A key released while the window is elsewhere never reports its
        // key-up, so drop every key as soon as we stop hearing them.
        let subscriptions = vec![
            cx.on_blur(&focus, window, |this, _, cx| this.release_keys(cx)),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.release_keys(cx);
                }
            }),
        ];

        let tick_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break; // The view is gone.
                }
            }
        });

        Self {
            fleet,
            rover: None,
            focus,
            keys: Keys::default(),
            turbo: false,
            limits: Limits::from_env(),
            assist: false,
            verdict: None,
            gamepads,
            gamepad_error,
            pad,
            twist: Twist::default(),
            stop_ticks: 0,
            stopping: None,
            _tick_task: tick_task,
            _subscriptions: subscriptions,
        }
    }

    /// Start driving `rover_id` and take the keyboard. `assist` picks the
    /// level: L1 with the obstacle assist, L0 without.
    pub fn drive(
        &mut self,
        rover_id: u64,
        assist: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rover = Some(rover_id);
        self.keys = Keys::default();
        self.set_assist(assist, cx);
        window.focus(&self.focus, cx);
    }

    /// Stop driving. The loop sends the rover its last zero commands.
    pub fn release(&mut self, cx: &mut Context<Self>) {
        self.set_assist(false, cx);
        self.rover = None;
        self.release_keys(cx);
    }

    /// Turn the obstacle assist on or off for the rover being driven.
    pub fn set_assist(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(rover_id) = self.rover else { return };
        if on && !self.assist {
            self.fleet.start_assist(rover_id);
        }
        if !on && self.assist {
            // The assist runs as a task on the rover; end it, so the plain
            // stick drives again.
            self.fleet.perform(rover_id, RoverAction::Clear);
        }
        self.assist = on;
        self.verdict = None;
        cx.notify();
    }

    /// The level the operator is driving at.
    pub fn level(&self) -> Level {
        if self.assist {
            Level::Assisted
        } else {
            Level::Manual
        }
    }

    /// The command sent on the last tick.
    pub fn twist(&self) -> Twist {
        self.twist
    }

    /// Give this to the element that should receive the drive keys
    /// (`.track_focus(..)`), together with [`Self::key_down`] and
    /// [`Self::key_up`].
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.keys.set(&event.keystroke.key, true) {
            cx.notify();
        }
    }

    pub fn key_up(&mut self, event: &KeyUpEvent, cx: &mut Context<Self>) {
        if self.keys.set(&event.keystroke.key, false) {
            cx.notify();
        }
    }

    fn release_keys(&mut self, cx: &mut Context<Self>) {
        self.keys = Keys::default();
        cx.notify();
    }

    // ------------------------------------------------------------ the loop

    /// One 20 Hz step: read the inputs, send a command.
    fn tick(&mut self, cx: &mut Context<Self>) {
        let before = (self.twist, self.verdict);

        self.handle_gamepad_buttons();
        let wanted = self.wanted_twist();
        match self.rover.filter(|_| self.assist) {
            Some(rover_id) => self.send_assisted(rover_id, wanted.unwrap_or_default()),
            None => self.send(wanted),
        }

        if (self.twist, self.verdict) != before {
            cx.notify();
        }
    }

    /// What the operator is asking for right now. `None`: hands off.
    fn wanted_twist(&self) -> Option<Twist> {
        self.rover?;
        if let Some(pad) = self.active_pad()
            && pad.is_pressed(Button::LeftTrigger)
        {
            let limit = self.limit(pad.is_pressed(Button::RightTrigger));
            // gilrs: stick up is +Y, stick right is +X. Turning left is +w.
            return Some(Twist {
                v: deadzone(pad.value(Axis::LeftStickY)) as f64 * limit.v,
                w: -deadzone(pad.value(Axis::RightStickX)) as f64 * limit.w,
            });
        }
        if self.keys.any() {
            let limit = self.limit(false);
            let axis = |plus: bool, minus: bool| (plus as i8 - minus as i8) as f64;
            return Some(Twist {
                v: axis(self.keys.forward, self.keys.back) * limit.v,
                w: axis(self.keys.left, self.keys.right) * limit.w,
            });
        }
        None
    }

    /// L1: pass the command through the obstacle assist and send it. The
    /// rover expects one every tick, so hands-off sends zeros.
    fn send_assisted(&mut self, rover_id: u64, wanted: Twist) {
        let seen = self
            .fleet
            .remote_state(rover_id)
            .filter(|(_, age)| *age <= assist::MAX_STATE_AGE_S);
        let (twist, verdict) = match seen {
            Some((state, _age)) => {
                let (v, w, verdict) = assist::filter(wanted.v, wanted.w, &state);
                (Twist { v, w }, Some(verdict))
            }
            // Blind: the assist cannot vouch for any motion.
            None => (Twist::default(), None),
        };
        self.fleet.drive_assisted(rover_id, twist.v, twist.w);
        self.twist = twist;
        self.verdict = verdict;
    }

    /// L0: send the command as it is, and nothing at all while hands-off.
    fn send(&mut self, wanted: Option<Twist>) {
        match (self.rover, wanted) {
            (Some(rover_id), Some(twist)) => {
                self.fleet.drive(rover_id, twist.v, twist.w);
                self.twist = twist;
                self.stopping = Some(rover_id);
                self.stop_ticks = STOP_TICKS;
            }
            _ => {
                self.twist = Twist::default();
                if let Some(rover_id) = self.stopping {
                    self.fleet.drive(rover_id, 0.0, 0.0);
                    self.stop_ticks = self.stop_ticks.saturating_sub(1);
                    if self.stop_ticks == 0 {
                        self.stopping = None;
                    }
                }
            }
        }
    }

    fn limit(&self, pad_turbo: bool) -> Twist {
        if self.turbo || pad_turbo {
            self.limits.turbo
        } else {
            self.limits.normal
        }
    }

    // ---------------------------------------------------------- the gamepad

    fn active_pad(&self) -> Option<gilrs::Gamepad<'_>> {
        let gamepads = self.gamepads.as_ref()?;
        gamepads.connected_gamepad(self.pad?)
    }

    /// Follow pads being plugged in and out, and act on button presses.
    fn handle_gamepad_buttons(&mut self) {
        let Some(gamepads) = &mut self.gamepads else {
            return;
        };
        let mut pressed = Vec::new();
        while let Some(event) = gamepads.next_event() {
            match event.event {
                EventType::Connected if self.pad.is_none() => self.pad = Some(event.id),
                EventType::Disconnected if self.pad == Some(event.id) => {
                    self.pad = gamepads.gamepads().next().map(|(id, _)| id);
                }
                EventType::ButtonPressed(button, _) => {
                    self.pad = Some(event.id);
                    pressed.push(button);
                }
                _ => {}
            }
        }

        let Some(rover_id) = self.rover else { return };
        for button in pressed {
            match button {
                Button::East => self.fleet.perform(rover_id, RoverAction::Stop),
                Button::Start => self.fleet.perform(rover_id, RoverAction::ReleaseStop),
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------ the view

    fn drive_link_panel(&self, window: &Window) -> Div {
        let has_keyboard = self.focus.is_focused(window) && window.is_window_active();
        let keyboard = if has_keyboard {
            ("held by this window".to_string(), theme::GREEN)
        } else {
            ("released · click to drive".to_string(), theme::ORANGE_TEXT)
        };

        let pad_name = self.active_pad().map(|pad| pad.name().to_string());
        let gamepad = match (&pad_name, &self.gamepad_error) {
            (Some(name), _) => (
                format!("{name} · hold {}", deadman_label(name)),
                theme::GREEN,
            ),
            (None, Some(error)) => (error.clone(), theme::ORANGE_TEXT),
            (None, None) => ("none".to_string(), theme::TEXT_MUTED),
        };

        let link = match self.fleet.link_state() {
            LinkState::Connected => ("connected".to_string(), theme::GREEN),
            LinkState::Connecting => ("connecting…".to_string(), theme::ORANGE_TEXT),
            LinkState::ConfigError(error) => (error, theme::RED.fill),
        };

        let rover = self.rover.and_then(|id| self.fleet.rover(id));
        let pose = match rover.as_ref().and_then(|rover| rover.pose.as_ref()) {
            Some(pose) if pose.tracked => ("tracking".to_string(), theme::GREEN),
            Some(_) => (
                "lost · the stick still drives".to_string(),
                theme::ORANGE_TEXT,
            ),
            None => ("unknown".to_string(), theme::TEXT_MUTED),
        };
        let task = match rover.as_ref().map(|rover| &rover.activity) {
            Some(Activity::HandDriven) => ("obstacle assist".to_string(), theme::GREEN),
            Some(Activity::Driving { .. }) => (
                "running · it overrides the stick".to_string(),
                theme::RED.fill,
            ),
            Some(Activity::Stopped) => ("e-stop engaged".to_string(), theme::RED.fill),
            Some(Activity::Idle) | None => ("none".to_string(), theme::TEXT_MUTED),
            Some(_) => ("on hold · goal kept".to_string(), theme::TEXT),
        };

        panel()
            .child(heading("Drive link"))
            .child(facts([
                ("Keyboard", keyboard.0, keyboard.1),
                ("Gamepad", gamepad.0, gamepad.1),
                ("Link", link.0, link.1),
                ("Pose", pose.0, pose.1),
                ("Task", task.0, task.1),
            ]))
            .child(note(
                "If this window loses focus, the keys are released and the rover stops.",
            ))
    }

    fn assist_panel(&self, cx: &mut Context<Self>) -> Div {
        let can_assist = self
            .rover
            .and_then(|id| self.fleet.rover(id))
            .is_some_and(|rover| rover.available_levels.contains(&Level::Assisted));

        let (status, status_color) = match (self.assist, self.verdict) {
            (false, _) if !can_assist => (
                "this rover cannot run the assist".to_string(),
                theme::TEXT_MUTED,
            ),
            (false, _) => (Level::Manual.description().to_string(), theme::TEXT_MUTED),
            (true, None) => (
                "holding · waiting for the rover's obstacle map".to_string(),
                theme::ORANGE_TEXT,
            ),
            (true, Some(Verdict::Clear)) => ("path clear".to_string(), theme::GREEN),
            (true, Some(Verdict::Slowed(fraction))) => (
                format!("obstacle ahead · {:.0}% of your command", fraction * 100.0),
                theme::ORANGE_TEXT,
            ),
            (true, Some(Verdict::Blocked)) => (
                "blocked · back up or turn away".to_string(),
                theme::RED.fill,
            ),
        };

        let choice = |id: &'static str, level: Level, on: bool| {
            let label = format!("{} ({})", if on { "On" } else { "Off" }, level.code());
            segment(id, label, self.assist == on)
                .on_click(cx.listener(move |this, _, _, cx| this.set_assist(on, cx)))
        };
        let switch = segmented().child(choice("assist-off", Level::Manual, false));
        let switch = if can_assist || self.assist {
            switch.child(choice("assist-on", Level::Assisted, true))
        } else {
            switch.child(segment("assist-on", "On (L1)", false).opacity(0.4))
        };

        panel()
            .child(heading("Obstacle assist"))
            .child(switch)
            .child(mono(status).text_color(rgb(status_color)))
    }

    fn speed_panel(&self, cx: &mut Context<Self>) -> Div {
        let choice = |id: &'static str, limit: Twist, turbo: bool, selected: bool| {
            let name = if turbo { "Turbo" } else { "Normal" };
            segment(id, format!("{name} · {:.1} m/s", limit.v), selected).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.turbo = turbo;
                    cx.notify();
                },
            ))
        };
        panel().child(heading("Speed")).child(
            segmented()
                .child(choice(
                    "speed-normal",
                    self.limits.normal,
                    false,
                    !self.turbo,
                ))
                .child(choice("speed-turbo", self.limits.turbo, true, self.turbo)),
        )
    }

    fn safety_panel(&self, cx: &mut Context<Self>) -> Div {
        let stopped_here = self
            .rover
            .and_then(|id| self.fleet.rover(id))
            .is_some_and(|rover| rover.stopped_here);
        let name = self
            .rover
            .map(|id| format!("Rover {id}"))
            .unwrap_or_else(|| "rover".into());

        let perform = |action: RoverAction| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, _, cx| {
                if let Some(rover_id) = this.rover {
                    this.fleet.perform(rover_id, action.clone());
                    cx.notify();
                }
            })
        };
        let release = if stopped_here {
            button(
                "release-stop",
                RoverAction::ReleaseStop.label(),
                ButtonLook::Outline,
            )
            .on_click(perform(RoverAction::ReleaseStop))
        } else {
            button(
                "release-stop",
                RoverAction::ReleaseStop.label(),
                ButtonLook::Disabled,
            )
        };

        panel()
            .child(heading("Safety"))
            .child(
                button("stop-rover", format!("Stop {name}"), ButtonLook::Stop)
                    .min_h(px(56.0))
                    .text_size(px(15.0))
                    .on_click(perform(RoverAction::Stop)),
            )
            .child(release)
    }
}

impl Render for DriveControls {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(self.drive_link_panel(window))
            .child(self.assist_panel(cx))
            .child(self.speed_panel(cx))
            .child(self.safety_panel(cx))
    }
}

impl Drop for DriveControls {
    fn drop(&mut self) {
        // Don't leave a rover coasting on its last command until the lease ends.
        if let Some(rover_id) = self.stopping {
            self.fleet.drive(rover_id, 0.0, 0.0);
        }
    }
}

/// Zero inside the dead zone, then rescaled so the output still spans 0..=1.
fn deadzone(value: f32) -> f32 {
    let size = value.abs();
    if !size.is_finite() || size < DEADZONE {
        return 0.0;
    }
    (value.signum() * (size - DEADZONE) / (1.0 - DEADZONE)).clamp(-1.0, 1.0)
}

/// The name printed on the pad's enable button. gilrs reports every pad in
/// the Xbox layout, so only the label differs.
fn deadman_label(pad_name: &str) -> &'static str {
    let name = pad_name.to_ascii_lowercase();
    let playstation = [
        "ps3",
        "ps4",
        "ps5",
        "dualshock",
        "dualsense",
        "sony",
        "playstation",
    ]
    .iter()
    .any(|brand| name.contains(brand));
    if playstation { "L1" } else { "LB" }
}
