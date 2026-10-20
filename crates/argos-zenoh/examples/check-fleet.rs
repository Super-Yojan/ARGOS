//! Read-only connectivity check. Credentials are read from environment, never printed.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::var("ARGOS_ZENOH_ENDPOINT")?;
    let user = std::env::var("ARGOS_ZENOH_USER").ok();
    let password = std::env::var("ARGOS_ZENOH_PASSWORD").ok();
    let client = argos_zenoh::Client::connect_authenticated(
        &endpoint,
        &std::env::var("ARGOS_ZENOH_PREFIX").unwrap_or_else(|_| "terra/phone".into()),
        user.as_deref().zip(password.as_deref()),
    )?;
    println!("Authenticated fleet connection established");
    if std::env::args().any(|a| a == "--status") {
        std::thread::sleep(std::time::Duration::from_secs(3));
        let snapshot = client.snapshot();
        println!("Fleet link: {}; discovered rovers: {}; notice: {:?}",
            snapshot.link, snapshot.rovers.len(), snapshot.notice);
        for rover in &snapshot.rovers {
            println!("Rover {}: membership={}, pose_present={}, pose_age={:?}",
                rover.id, rover.membership, rover.pose.is_some(), rover.pose_age);
        }
        let state: serde_json::Value = serde_json::from_str(&client.operator_snapshot())?;
        if let Some(rovers) = state.as_object() {
            for (id, rover) in rovers {
                let a = &rover["authority"];
                if let Ok(id) = id.parse::<u64>() {
                    let grid = client.occupancy_grid(id);
                    println!(
                        "{}",
                        serde_json::json!({"id":id.to_string(),"occupancy":grid.map(|(g,age,_)|serde_json::json!({"width":g.width,"height":g.height,"age":age,"observed_cells":g.occupancy.iter().filter(|&&v|v>=0).count()}))})
                    );
                }
                println!(
                    "{}",
                    serde_json::json!({"id":id,"safety":a["safety"],"reason":a["reason"],"age":a["age"],"level":a["requested_level"],"hardware":rover["hardware"]})
                );
            }
        }
    }
    client.disconnect();
    Ok(())
}
