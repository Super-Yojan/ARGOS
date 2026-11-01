//! One program for a mocap velocity session: records OptiTrack (`optitrack/pose`)
//! and the rovers' published poses for the whole session, and drives the runs.
//!
//! ```sh
//! cargo run --release -p argos-zenoh --example mocap_session
//! ```
//!
//! It writes `mocap-<UTC>/mocap.jsonl` and `mocap-<UTC>/drive_log.csv`, both on
//! this computer's clock, which is what `analyze_velocity.py --live` reads.
//! Commands at the prompt:
//!
//! ```text
//! d SPEED DIST [left|right] [TAKE]   drive at SPEED m/s for DIST m (straight, or an arc)
//! s SECONDS [TAKE]                   mark a still window (no command is sent)
//! r                                  show the stream rates
//! q                                  stop recording and quit
//! ```
//!
//! Takes are named automatically (`B_v0.3_r1`, `E_left_r1`, `A_static_r1`) unless
//! TAKE is given; a repeat number is added to a given TAKE that has none.
//! Connection settings are the dashboard's `ARGOS_ZENOH_*` variables
//! (environment, then `./.env`). Killing the program stops the rover through
//! Terra's 0.5 s command freshness limit.
use argos_core::TeleopLimits;
use argos_zenoh::Client;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zenoh::Wait;

type AnyError = Box<dyn std::error::Error + Send + Sync>;
const SETTLE_SECONDS: f64 = 3.0;

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

struct Rates {
    mocap: AtomicU64,
    rover: AtomicU64,
    since: std::sync::Mutex<Instant>,
}

impl Rates {
    fn line(&self) -> String {
        let mut since = self.since.lock().unwrap();
        let secs = since.elapsed().as_secs_f64().max(1e-3);
        *since = Instant::now();
        let (m, r) = (
            self.mocap.swap(0, Ordering::Relaxed),
            self.rover.swap(0, Ordering::Relaxed),
        );
        format!(
            "OptiTrack {:.0} Hz, rover pose {:.1} Hz",
            m as f64 / secs,
            r as f64 / secs
        )
    }
}

/// Record OptiTrack and rover poses on a second zenoh session; returns its handles.
fn start_recorder(
    endpoint: &str,
    prefix: &str,
    credentials: Option<(&str, &str)>,
    key: &str,
    out: &Path,
    rates: Arc<Rates>,
) -> Result<(zenoh::Session, Vec<zenoh::pubsub::Subscriber<()>>), AnyError> {
    let mut config = zenoh::Config::default();
    config.insert_json5("mode", "\"client\"")?;
    config.insert_json5(
        "connect/endpoints",
        &serde_json::to_string(&vec![endpoint])?,
    )?;
    config.insert_json5("scouting/multicast/enabled", "false")?;
    if let Some((user, password)) = credentials {
        config.insert_json5("transport/auth/usrpwd/user", &serde_json::to_string(user)?)?;
        config.insert_json5(
            "transport/auth/usrpwd/password",
            &serde_json::to_string(password)?,
        )?;
    }
    let session = zenoh::open(config).wait()?;
    let (tx, rx) = mpsc::channel::<String>();
    let mut file = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(out)?,
    );
    std::thread::spawn(move || {
        let mut flushed = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => {
                    let _ = writeln!(file, "{line}");
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if flushed.elapsed() >= Duration::from_millis(500) {
                let _ = file.flush();
                flushed = Instant::now();
            }
        }
        let _ = file.flush();
    });
    let start = Instant::now();
    let make = |is_mocap: bool| {
        let (tx, rates) = (tx.clone(), rates.clone());
        move |sample: zenoh::sample::Sample| {
            let recv_mono_ns = start.elapsed().as_nanos() as u64;
            let recv_utc_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            let bytes = sample.payload().to_bytes();
            let text = String::from_utf8_lossy(&bytes);
            let payload = serde_json::from_str::<Value>(&text)
                .unwrap_or_else(|_| Value::String(text.into_owned()));
            let counter = if is_mocap { &rates.mocap } else { &rates.rover };
            counter.fetch_add(1, Ordering::Relaxed);
            let _ = tx.send(
                json!({"recv_utc_ns": recv_utc_ns, "recv_mono_ns": recv_mono_ns,
                       "key": sample.key_expr().as_str(), "payload": payload})
                .to_string(),
            );
        }
    };
    let subscribers = vec![
        session
            .declare_subscriber(key)
            .callback(make(true))
            .wait()?,
        session
            .declare_subscriber(format!("{prefix}/*/pose"))
            .callback(make(false))
            .wait()?,
    ];
    Ok((session, subscribers))
}

#[allow(clippy::too_many_arguments)]
fn append_log(
    path: &Path,
    take: &str,
    rover: u64,
    speed: f64,
    distance: f64,
    turn: &str,
    utc_start: f64,
    utc_end: f64,
    phone_distance: f64,
    note: &str,
) -> std::io::Result<()> {
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
        "{take},{rover},{speed},{distance},{turn},{utc_start:.3},{utc_end:.3},{:.3},{phone_distance:.3},{note}",
        utc_end - utc_start
    )
}

/// `base_rN` with the next free N, unless the name already ends in `_rN`.
fn take_name(base: &str, used: &mut BTreeMap<String, u32>) -> String {
    if let Some((stem, n)) = base.rsplit_once("_r")
        && !stem.is_empty()
        && n.parse::<u32>().is_ok()
    {
        return base.to_string();
    }
    let n = used.entry(base.to_string()).or_insert(0);
    *n += 1;
    format!("{base}_r{n}")
}

fn set_teleop(c: &Client, id: u64) -> Result<(), AnyError> {
    let token = c.action(id, "autonomy", json!({"level": "teleop"}))?;
    let start = Instant::now();
    loop {
        let states: Value = serde_json::from_str(&c.operator_snapshot())?;
        let s = &states[id.to_string()]["status"];
        if s["token"] == token && s["result"] == "accepted" && s["safety"] == "clear" {
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(10) {
            return Err(format!("rover {id} did not accept L0 teleop: {s}").into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Hold forward (and a turn key) for distance / speed seconds, then release.
fn drive(
    c: &Client,
    id: u64,
    speed: f64,
    distance: f64,
    turn: Option<&str>,
) -> Result<(f64, f64, f64, &'static str), AnyError> {
    c.set_teleop_guard(TeleopLimits {
        linear: speed,
        ..TeleopLimits::default()
    });
    let origin = pose(c, id);
    let duration = distance / speed;
    let press = Instant::now();
    loop {
        match c.input(id, "w", true) {
            Ok(()) => break,
            Err(e) if press.elapsed() > Duration::from_secs(3) => return Err(e.into()),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let utc_start = utc();
    if let Some(key) = turn {
        c.input(id, key, true)?;
    }
    let driving = Instant::now();
    let mut note = "";
    while driving.elapsed().as_secs_f64() < duration {
        c.refresh_drive(id);
        // Safety net on the phone's own pose; the run itself is timed.
        if turn.is_none()
            && let (Some(a), Some(b)) = (origin, pose(c, id))
            && (b.0 - a.0).hypot(b.1 - a.1) > distance + 0.5
        {
            note = "stopped early: phone pose passed the distance by 0.5 m";
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(key) = turn {
        let _ = c.input(id, key, false);
    }
    let _ = c.input(id, "w", false);
    c.clear_input();
    let utc_end = utc();
    std::thread::sleep(Duration::from_secs_f64(SETTLE_SECONDS));
    let moved = match (origin, pose(c, id)) {
        (Some(a), Some(b)) => (b.0 - a.0).hypot(b.1 - a.1),
        _ => f64::NAN,
    };
    Ok((utc_start, utc_end, moved, note))
}

fn main() -> Result<(), AnyError> {
    let mut args = std::env::args().skip(1);
    let (mut rover, mut key, mut dir) = (None, "optitrack/pose".to_string(), None);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or("usage: mocap_session [--rover ID] [--key optitrack/pose] [--dir FOLDER]")
        };
        match arg.as_str() {
            "--rover" => rover = Some(value()?.parse::<u64>()?),
            "--key" => key = value()?.to_string(),
            "--dir" => dir = Some(PathBuf::from(value()?)),
            _ => {
                return Err(
                    "usage: mocap_session [--rover ID] [--key optitrack/pose] [--dir FOLDER]"
                        .into(),
                );
            }
        }
    }
    let dir = dir.unwrap_or_else(|| PathBuf::from(format!("mocap-{}", utc() as u64)));
    std::fs::create_dir_all(&dir)?;
    let log_path = dir.join("drive_log.csv");

    let endpoint = setting("ARGOS_ZENOH_ENDPOINT")
        .unwrap_or_else(|| "tls/argos.34.135.204.210.sslip.io:7448".into());
    let prefix = setting("ARGOS_ZENOH_PREFIX").unwrap_or_else(|| "terra/phone".into());
    let user = setting("ARGOS_ZENOH_USER").unwrap_or_else(|| "terra".into());
    let password = setting("ARGOS_ZENOH_PASSWORD").filter(|p| !p.is_empty());
    let credentials = password.as_deref().map(|p| (user.as_str(), p));

    println!("connecting to {endpoint} ({prefix})");
    let rates = Arc::new(Rates {
        mocap: AtomicU64::new(0),
        rover: AtomicU64::new(0),
        since: std::sync::Mutex::new(Instant::now()),
    });
    let recorder = start_recorder(
        &endpoint,
        &prefix,
        credentials,
        &key,
        &dir.join("mocap.jsonl"),
        rates.clone(),
    )?;
    let c = Client::connect_authenticated(&endpoint, &prefix, credentials)?;
    let start = Instant::now();
    while c.snapshot().link != "healthy" {
        if start.elapsed() > Duration::from_secs(10) {
            return Err(format!("router link is {}", c.snapshot().link).into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(2000));
    println!("recording to {} ({})", dir.display(), rates.line());
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
    set_teleop(&c, id)?;
    println!(
        "rover {id} at L0 teleop. Commands: d SPEED DIST [left|right] [TAKE] | s SECONDS [TAKE] | r | q"
    );

    let mut used = BTreeMap::new();
    let stdin = std::io::stdin();
    loop {
        print!("> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let result: Result<(), AnyError> = (|| {
            match words.as_slice() {
                [] => Ok(()),
                ["q" | "quit"] => Err("quit".into()),
                ["r" | "rates"] => {
                    println!("{}", rates.line());
                    Ok(())
                }
                ["s" | "still", seconds, rest @ ..] => {
                    let seconds: f64 = seconds.parse()?;
                    if !(1.0..=3600.).contains(&seconds) {
                        return Err("still takes 1..=3600 seconds".into());
                    }
                    let take = take_name(rest.first().copied().unwrap_or("A_static"), &mut used);
                    println!("{take}: keep the rover still for {seconds} s");
                    let utc_start = utc();
                    std::thread::sleep(Duration::from_secs_f64(seconds));
                    append_log(
                        &log_path,
                        &take,
                        id,
                        0.,
                        0.,
                        "still",
                        utc_start,
                        utc(),
                        f64::NAN,
                        "",
                    )?;
                    println!("{take} done ({})", rates.line());
                    Ok(())
                }
                ["d" | "drive", speed, distance, rest @ ..] => {
                    let (speed, distance): (f64, f64) = (speed.parse()?, distance.parse()?);
                    if !(0.05..=0.5).contains(&speed) || !(0.1..=20.).contains(&distance) {
                        return Err("speed must be 0.05..=0.5 m/s and distance 0.1..=20 m".into());
                    }
                    let (turn, rest) = match rest.first() {
                        Some(&"left") => (Some("left"), &rest[1..]),
                        Some(&"right") => (Some("right"), &rest[1..]),
                        _ => (None, rest),
                    };
                    let base = match (rest.first(), turn) {
                        (Some(t), _) => t.to_string(),
                        (None, Some(dir)) => format!("E_{dir}"),
                        (None, None) => format!("B_v{speed}"),
                    };
                    let take = take_name(&base, &mut used);
                    let key = turn.map(|t| if t == "left" { "a" } else { "d" });
                    println!(
                        "{take}: {} at {speed} m/s for {distance} m ({:.1} s)",
                        turn.map_or("straight".to_string(), |t| format!("arc {t}")),
                        distance / speed
                    );
                    let (utc_start, utc_end, moved, note) =
                        match drive(&c, id, speed, distance, key) {
                            Ok(v) => v,
                            Err(e) => {
                                // A refused drive usually means L0 was lost; ask for it again once.
                                println!("drive refused ({e}); setting L0 again");
                                set_teleop(&c, id)?;
                                drive(&c, id, speed, distance, key)?
                            }
                        };
                    append_log(
                        &log_path,
                        &take,
                        id,
                        speed,
                        distance,
                        turn.unwrap_or("none"),
                        utc_start,
                        utc_end,
                        moved,
                        note,
                    )?;
                    println!(
                        "{take} done: phone pose moved {moved:.2} m{} ({})",
                        if note.is_empty() {
                            String::new()
                        } else {
                            format!(", {note}")
                        },
                        rates.line()
                    );
                    Ok(())
                }
                _ => Err(
                    "commands: d SPEED DIST [left|right] [TAKE] | s SECONDS [TAKE] | r | q".into(),
                ),
            }
        })();
        match result {
            Ok(()) => {}
            Err(e) if e.to_string() == "quit" => break,
            Err(e) => println!("{e}"),
        }
    }
    c.clear_input();
    c.disconnect();
    drop(recorder);
    std::thread::sleep(Duration::from_millis(600));
    println!("saved {}", dir.display());
    Ok(())
}
