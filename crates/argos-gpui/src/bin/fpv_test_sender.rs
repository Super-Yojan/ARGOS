//! Stand-in for TerraPhone. It waits for a dashboard `start` and streams a
//! generated H.264 pattern over WebRTC. It never publishes a drive command.
//!
//! ```text
//! cargo run -p argos-gpui --bin fpv-test-sender -- --endpoint tcp/127.0.0.1:7447 --id 7
//! ```
//!
//! Set `ARGOS_FPV_STUN=none` for a same-machine router so ICE stays on host
//! candidates. TURN variables match the dashboard. A password, when the router
//! needs one, comes from the environment or `.env` and is not printed.

use argos_gpui::{FpvConfig, SenderOptions, run_test_sender};
use std::collections::BTreeMap;
use std::env;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

fn main() {
    let _ = dotenvy::dotenv();
    let mut endpoint =
        env::var("ARGOS_ZENOH_ENDPOINT").unwrap_or_else(|_| "tcp/127.0.0.1:7447".to_string());
    let mut prefix = env::var("ARGOS_ZENOH_PREFIX").unwrap_or_else(|_| "terra/phone".to_string());
    let mut phone_id = 1u64;
    let mut announce = true;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--endpoint" => endpoint = required(&mut args, "--endpoint"),
            "--prefix" => prefix = required(&mut args, "--prefix"),
            "--id" => {
                let raw = required(&mut args, "--id");
                phone_id = raw
                    .parse()
                    .unwrap_or_else(|_| usage_exit("phone id must be a number"));
            }
            "--no-announce" => announce = false,
            "--help" | "-h" => {
                print_usage();
                return;
            }
            other => usage_exit(&format!("unknown argument {other}")),
        }
    }
    let mut values = BTreeMap::new();
    for key in [
        "ARGOS_FPV_STUN",
        "ARGOS_FPV_TURN_URL",
        "ARGOS_FPV_TURN_USERNAME",
        "ARGOS_FPV_TURN_CREDENTIAL",
    ] {
        if let Ok(value) = env::var(key) {
            values.insert(key.to_string(), value);
        }
    }
    let ice = FpvConfig::from_values(&values).unwrap_or_else(|error| {
        eprintln!("fpv-test-sender: {error}");
        std::process::exit(1);
    });
    let user = env::var("ARGOS_ZENOH_USER")
        .ok()
        .filter(|user| !user.is_empty());
    let password = env::var("ARGOS_ZENOH_PASSWORD")
        .ok()
        .filter(|password| !password.is_empty());
    if user.is_some() != password.is_some() {
        eprintln!(
            "fpv-test-sender: ARGOS_ZENOH_USER and ARGOS_ZENOH_PASSWORD must both be set, or neither"
        );
        std::process::exit(1);
    }
    if let Err(error) = run_test_sender(SenderOptions {
        endpoint,
        prefix,
        phone_id,
        ice,
        announce,
        user,
        password,
        cancel: Arc::new(AtomicBool::new(false)),
    }) {
        eprintln!("fpv-test-sender: {error}");
        std::process::exit(1);
    }
}

fn required(args: &mut impl Iterator<Item = String>, name: &str) -> String {
    args.next()
        .unwrap_or_else(|| usage_exit(&format!("{name} needs a value")))
}

fn usage_exit(reason: &str) -> ! {
    eprintln!("fpv-test-sender: {reason}");
    print_usage();
    std::process::exit(2);
}

fn print_usage() {
    eprintln!(
        "\
fpv-test-sender publishes an H.264 test pattern when the dashboard sends start.

  --endpoint tcp/127.0.0.1:7447   Zenoh router (or ARGOS_ZENOH_ENDPOINT)
  --prefix terra/phone            Key prefix (or ARGOS_ZENOH_PREFIX)
  --id 1                          Phone id the dashboard will start
  --no-announce                   Do not publish fleet/state and pose

ICE: ARGOS_FPV_STUN (default stun:stun.l.google.com:19302, or none),
ARGOS_FPV_TURN_URL, ARGOS_FPV_TURN_USERNAME, ARGOS_FPV_TURN_CREDENTIAL.
Ctrl-C stops the process. This program does not publish teleop or hardware."
    );
}
