//! Record live OptiTrack poses and the rovers' published poses to JSON lines,
//! stamped on this computer's clock, for the mocap velocity experiment.
//!
//! ```sh
//! cargo run --release -p argos-zenoh --example mocap_record -- --out mocap.jsonl
//! ```
//!
//! Leave it running for the whole session. Each line is
//! `{"recv_utc_ns", "recv_mono_ns", "key", "payload"}`, with `payload` parsed as
//! JSON when it is JSON and kept as text otherwise. `drive_distance` writes its
//! run times on the same clock, so takes are cut from this file afterwards and
//! no Motive export or sync jerk is needed. Connection settings are the
//! dashboard's `ARGOS_ZENOH_*` variables (environment, then `./.env`);
//! `--endpoint` overrides the router if the OptiTrack node uses another one.
use serde_json::{Value, json};
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zenoh::Wait;

const USAGE: &str = "usage: mocap_record --out FILE [--key optitrack/pose] [--endpoint tcp/HOST:PORT] [--no-rover-pose]";

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

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut args = std::env::args().skip(1);
    let (mut out, mut key, mut endpoint, mut rover_pose) =
        (None, "optitrack/pose".to_string(), None, true);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(USAGE);
        match arg.as_str() {
            "--out" => out = Some(value()?),
            "--key" => key = value()?,
            "--endpoint" => endpoint = Some(value()?),
            "--no-rover-pose" => rover_pose = false,
            _ => return Err(USAGE.into()),
        }
    }
    let out = out.ok_or(USAGE)?;
    let endpoint = endpoint
        .or_else(|| setting("ARGOS_ZENOH_ENDPOINT"))
        .unwrap_or_else(|| "tls/argos.34.135.204.210.sslip.io:7448".into());
    let prefix = setting("ARGOS_ZENOH_PREFIX").unwrap_or_else(|| "terra/phone".into());
    let user = setting("ARGOS_ZENOH_USER").unwrap_or_else(|| "terra".into());
    let password = setting("ARGOS_ZENOH_PASSWORD").filter(|p| !p.is_empty());

    let mut config = zenoh::Config::default();
    config.insert_json5("mode", "\"client\"")?;
    config.insert_json5(
        "connect/endpoints",
        &serde_json::to_string(&vec![&endpoint])?,
    )?;
    config.insert_json5("scouting/multicast/enabled", "false")?;
    if let Some(password) = &password {
        config.insert_json5("transport/auth/usrpwd/user", &serde_json::to_string(&user)?)?;
        config.insert_json5(
            "transport/auth/usrpwd/password",
            &serde_json::to_string(password)?,
        )?;
    }
    println!("connecting to {endpoint}");
    let session = zenoh::open(config).wait()?;

    let start = Instant::now();
    let (tx, rx) = mpsc::channel::<String>();
    let record = move |sample: zenoh::sample::Sample| {
        let recv_mono_ns = start.elapsed().as_nanos() as u64;
        let recv_utc_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let bytes = sample.payload().to_bytes();
        let text = String::from_utf8_lossy(&bytes);
        let payload = serde_json::from_str::<Value>(&text)
            .unwrap_or_else(|_| Value::String(text.into_owned()));
        let line = json!({"recv_utc_ns": recv_utc_ns, "recv_mono_ns": recv_mono_ns,
                          "key": sample.key_expr().as_str(), "payload": payload});
        let _ = tx.send(line.to_string());
    };
    let mut subscribers = vec![
        session
            .declare_subscriber(key.as_str())
            .callback(record.clone())
            .wait()?,
    ];
    if rover_pose {
        subscribers.push(
            session
                .declare_subscriber(format!("{prefix}/*/pose"))
                .callback(record)
                .wait()?,
        );
    }

    let mut file = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&out)?,
    );
    println!(
        "recording {key}{} to {out}; Ctrl-C to stop",
        if rover_pose {
            format!(" and {prefix}/*/pose")
        } else {
            String::new()
        }
    );
    let (mut mocap, mut rover, mut last_report, mut last_flush) =
        (0u64, 0u64, Instant::now(), Instant::now());
    let mut first = true;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                if line.contains(&format!("\"key\":\"{key}\"")) {
                    mocap += 1;
                    if first {
                        println!("first OptiTrack message: {line}");
                        first = false;
                    }
                } else {
                    rover += 1;
                }
                writeln!(file, "{line}")?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if last_flush.elapsed() >= Duration::from_millis(500) {
            file.flush()?;
            last_flush = Instant::now();
        }
        if last_report.elapsed() >= Duration::from_secs(5) {
            let secs = last_report.elapsed().as_secs_f64();
            println!(
                "{:>7.0} s  OptiTrack {:6.1} Hz  rover pose {:5.1} Hz",
                start.elapsed().as_secs_f64(),
                mocap as f64 / secs,
                rover as f64 / secs
            );
            if mocap == 0 {
                println!("         no OptiTrack messages on {key}; check the node and --endpoint");
            }
            (mocap, rover, last_report) = (0, 0, Instant::now());
        }
    }
    drop(subscribers);
    Ok(())
}
