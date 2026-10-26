//! Drive one rover at a fixed teleop speed for a fixed distance, for the
//! OptiTrack velocity experiment.
//!
//! ```sh
//! cargo run --release -p argos-zenoh --example drive_distance -- --speed 0.3 --distance 4 --take B_v0.3_r1
//! ```
//!
//! Connection settings are the dashboard's `ARGOS_ZENOH_*` variables, read from
//! the environment and then `./.env`. The script sets L0 teleop, waits for
//! Enter, holds forward for `distance / speed` seconds through the dashboard's
//! teleop guard (so the published speed is exactly `--speed`), releases, and
//! appends the run to `drive_log.csv`. The distance is open-loop in time: the
//! rover covers a little less while it accelerates, and the mocap measures the
//! real distance. Killing the script stops the rover through Terra's 0.5 s
//! command freshness limit.
use argos_core::TeleopLimits;
use argos_zenoh::Client;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const USAGE: &str = "usage: drive_distance --speed M_PER_S --distance M [--turn left|right] \
[--rover ID] [--take NAME] [--settle S] [--yes]";

fn setting(key: &str) -> Option<String> {
    if let Ok(value) = std::env::var(key) {
        return Some(value);
    }
    let file = std::fs::read_to_string(".env").ok()?;
    file.lines().find_map(|line| {
        let (k, v) = line
            .trim()
            .strip_prefix("export ")
            .unwrap_or(line.trim())
            .split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').trim_matches('\'').to_string())
    })
}

fn utc() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn pose(c: &Client, id: u64) -> Option<(f64, f64)> {
    let snapshot = c.snapshot();
    let rover = snapshot.rovers.iter().find(|r| r.id == id)?;
    rover.pose.as_ref().map(|p| (p.x, p.y))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (mut speed, mut distance, mut turn, mut rover, mut take, mut settle, mut yes) =
        (None, None, None, None, String::new(), 3.0, false);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(USAGE);
        match arg.as_str() {
            "--speed" => speed = Some(value()?.parse::<f64>()?),
            "--distance" => distance = Some(value()?.parse::<f64>()?),
            "--turn" => turn = Some(value()?),
            "--rover" => rover = Some(value()?.parse::<u64>()?),
            "--take" => take = value()?,
            "--settle" => settle = value()?.parse::<f64>()?,
            "--yes" => yes = true,
            _ => return Err(USAGE.into()),
        }
    }
    let (Some(speed), Some(distance)) = (speed, distance) else {
        return Err(USAGE.into());
    };
    // ARGOS teleop publishes at most 0.5 m/s; Terra enforces its own limits too.
    if !(0.05..=0.5).contains(&speed) || !(0.1..=20.).contains(&distance) {
        return Err("speed must be 0.05..=0.5 m/s and distance 0.1..=20 m".into());
    }
    let turn_key = match turn.as_deref() {
        None => None,
        Some("left") => Some("a"),
        Some("right") => Some("d"),
        Some(_) => return Err("--turn is left or right".into()),
    };
    let duration = distance / speed;

    let endpoint = setting("ARGOS_ZENOH_ENDPOINT")
        .unwrap_or_else(|| "tls/argos.34.135.204.210.sslip.io:7448".into());
    let prefix = setting("ARGOS_ZENOH_PREFIX").unwrap_or_else(|| "terra/phone".into());
    let user = setting("ARGOS_ZENOH_USER").unwrap_or_else(|| "terra".into());
    let password = setting("ARGOS_ZENOH_PASSWORD").filter(|p| !p.is_empty());
    println!("connecting to {endpoint} ({prefix})");
    let c = Client::connect_authenticated(
        &endpoint,
        &prefix,
        password.as_deref().map(|p| (user.as_str(), p)),
    )?;
    let start = Instant::now();
    while c.snapshot().link != "healthy" {
        if start.elapsed() > Duration::from_secs(10) {
            return Err(format!("router link is {}", c.snapshot().link).into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // Rover discovery arrives with the first fleet message.
    std::thread::sleep(Duration::from_millis(1500));
    let online: Vec<u64> = c
        .snapshot()
        .rovers
        .iter()
        .filter(|r| r.membership == "online")
        .map(|r| r.id)
        .collect();
    let id = match rover {
        Some(id) if online.contains(&id) => id,
        Some(id) => return Err(format!("rover {id} is not online (online: {online:?})").into()),
        None if online.len() == 1 => online[0],
        None => return Err(format!("pick a rover with --rover (online: {online:?})").into()),
    };

    c.set_teleop_guard(TeleopLimits {
        linear: speed,
        ..TeleopLimits::default()
    });
    let token = c.action(id, "autonomy", json!({"level": "teleop"}))?;
    let start = Instant::now();
    loop {
        let states: Value = serde_json::from_str(&c.operator_snapshot())?;
        let s = &states[id.to_string()]["status"];
        if s["token"] == token && s["result"] == "accepted" && s["safety"] == "clear" {
            break;
        }
        if start.elapsed() > Duration::from_secs(10) {
            return Err(format!("rover {id} did not accept L0 teleop: {s}").into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let what = match turn_key {
        Some(_) => format!("arc {} at", turn.as_deref().unwrap_or_default()),
        None => "straight at".into(),
    };
    println!(
        "rover {id}: {what} {speed} m/s for {distance} m ({duration:.1} s). \
         Start the Motive take{} and do the sync jerks.",
        if take.is_empty() {
            String::new()
        } else {
            format!(" {take}")
        }
    );
    if !yes {
        print!("Press Enter to drive (Ctrl-C to abort) ");
        std::io::stdout().flush()?;
        std::io::stdin().lock().read_line(&mut String::new())?;
    }

    let origin = pose(&c, id);
    let utc_start = utc();
    let press = Instant::now();
    loop {
        match c.input(id, "w", true) {
            Ok(()) => break,
            Err(e) if press.elapsed() > Duration::from_secs(3) => return Err(e.into()),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    if let Some(key) = turn_key {
        c.input(id, key, true)?;
    }
    let driving = Instant::now();
    let mut stopped_early = None;
    while driving.elapsed().as_secs_f64() < duration {
        c.refresh_drive(id);
        // Safety net on the phone's own pose; the run itself is timed.
        if turn_key.is_none()
            && let (Some(a), Some(b)) = (origin, pose(&c, id))
            && (b.0 - a.0).hypot(b.1 - a.1) > distance + 0.5
        {
            stopped_early = Some("phone pose passed the distance by 0.5 m");
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let held = driving.elapsed().as_secs_f64();
    if let Some(key) = turn_key {
        let _ = c.input(id, key, false);
    }
    let _ = c.input(id, "w", false);
    c.clear_input();
    let utc_end = utc();
    if let Some(reason) = stopped_early {
        println!("stopped early: {reason}");
    }
    std::thread::sleep(Duration::from_secs_f64(settle.clamp(0., 30.)));
    let vio = match (origin, pose(&c, id)) {
        (Some(a), Some(b)) => (b.0 - a.0).hypot(b.1 - a.1),
        _ => f64::NAN,
    };
    println!(
        "released after {held:.2} s. Phone pose moved {vio:.2} m. \
         Do the sync jerks and stop the Motive take."
    );

    let path = std::path::Path::new("drive_log.csv");
    let new = !path.exists();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    if new {
        writeln!(
            log,
            "take,rover,speed,distance,turn,utc_start,utc_end,held_s,phone_distance_m,note"
        )?;
    }
    writeln!(
        log,
        "{take},{id},{speed},{distance},{},{utc_start:.3},{utc_end:.3},{held:.3},{vio:.3},{}",
        turn.as_deref().unwrap_or("none"),
        stopped_early.unwrap_or("")
    )?;
    c.disconnect();
    Ok(())
}
