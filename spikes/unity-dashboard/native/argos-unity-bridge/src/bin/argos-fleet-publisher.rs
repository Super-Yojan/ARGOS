//! Fake fleet on the real ARGOS topics.
//!
//! Without Zorvane (this process is the listener the Unity bridge connects to):
//!
//! ```text
//! cargo run -p argos-unity-bridge --bin argos-fleet-publisher -- --count 8 --listen
//! ```
//!
//! Against Zorvane, do not run this publisher. Zorvane already owns `fleet/state`.
//! From a Zorvane checkout:
//!
//! ```text
//! TERRA_ROVER_COUNT=8 TERRA_TILES=1 TERRA_TILES_FETCH=0 cargo run -p zorvane
//! ```
//!
//! Then point Unity at `tcp/127.0.0.1:7447` and prefix `terra/rover`.
//! `--connect` exists only to inject the same fake samples onto an already
//! running router. It overwrites `fleet/state`. Prefer `TERRA_ROVER_COUNT`.

use argos_unity_bridge::{connect_publisher, serve};
use std::thread;
use std::time::Duration;

fn main() {
    let mut count = 8u64;
    let mut endpoint = "tcp/127.0.0.1:7447".to_string();
    let mut prefix = "terra/rover".to_string();
    let mut listen = true;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print_help();
                return;
            }
            "--listen" => listen = true,
            "--connect" => listen = false,
            "--count" => count = expect(&mut args, "--count").parse().unwrap_or(0),
            "--endpoint" => endpoint = expect(&mut args, "--endpoint"),
            "--prefix" => prefix = expect(&mut args, "--prefix"),
            other => {
                eprintln!("unknown argument: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }
    let publisher = if listen {
        serve(&endpoint, &prefix, count)
    } else {
        eprintln!(
            "ARGOS publisher connect mode writes fleet/state on {endpoint}. Stop this if Zorvane is the fleet source; use TERRA_ROVER_COUNT instead."
        );
        connect_publisher(&endpoint, &prefix, count)
    };
    if let Err(error) = publisher {
        eprintln!("ARGOS publisher failed: {error}");
        std::process::exit(1);
    }
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

fn expect(args: &mut impl Iterator<Item = String>, flag: &str) -> String {
    args.next().unwrap_or_else(|| {
        eprintln!("missing value for {flag}");
        std::process::exit(2);
    })
}

fn print_help() {
    eprintln!(
        "\
argos-fleet-publisher

Publishes terra-01..terra-N on the ARGOS rover topics.
  fleet/state, <id>/pose, <id>/camera/depth, <id>/map/occupancy
  terra-02 autonomy/status safety=hold (attention)
  terra-03 autonomy/status safety=emergency_stop (critical)
  Each occupancy grid is schema 1, 24x24 cells at 0.5 m, centered on that rover.
  Unknown outside a short radius, free around the rover, occupied walls to the north and west.

Options:
  --count N                 1..=32, default 8
  --endpoint tcp/host:port  default tcp/127.0.0.1:7447
  --prefix KEY              default terra/rover
  --listen                  bind that endpoint (default; no Zorvane required)
  --connect                 publish as a client onto an existing router

Zorvane, several rovers, no fake publisher:
  TERRA_ROVER_COUNT=8 TERRA_TILES=1 TERRA_TILES_FETCH=0 cargo run -p zorvane
  Unity: -argosSource zenoh -argosEndpoint tcp/127.0.0.1:7447
"
    );
}
