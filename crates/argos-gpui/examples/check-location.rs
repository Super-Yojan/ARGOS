//! Read-only check of the exact rover positions used by the GPUI map.
use argos_gpui::{DashboardConfig, FleetFeed};
fn main() -> Result<(), String> {
    let feed = FleetFeed::start(DashboardConfig::from_env()?);
    std::thread::sleep(std::time::Duration::from_secs(5));
    let picture = feed.picture();
    println!("{}: {}", picture.link, picture.detail);
    for mark in picture.marks {
        println!(
            "Control: requested={:?}, effective={:?}, safety={}, outdoor={:?}, hardware={:?}",
            mark.requested_level, mark.effective_level, mark.safety, mark.outdoor, mark.hardware
        );
        println!(
            "Rover {}: map=({}, {}), aligned={}, GPS={:?}",
            mark.id,
            mark.latitude,
            mark.longitude,
            mark.map_frame.is_some(),
            mark.raw_gps
        );
    }
    Ok(())
}
