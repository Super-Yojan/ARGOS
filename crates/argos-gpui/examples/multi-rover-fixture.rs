//! Read-only synthetic telemetry for reproducing multi-rover rendering failures.
use zenoh::Wait;
fn main() {
    let mut config = zenoh::Config::default();
    config.insert_json5("mode", "\"router\"").unwrap();
    config.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:17447\"]").unwrap();
    config.insert_json5("scouting/multicast/enabled", "false").unwrap();
    let session = zenoh::open(config).wait().unwrap();
    for tick in 0..1200 {
        let count = if tick < 50 { 1 } else { 2 };
        let ids: Vec<_> = (0..count).collect();
        session.put("terra/rover/fleet/state", serde_json::json!({"count":count,"max_count":32,"ids":ids}).to_string()).wait().unwrap();
        for id in ids {
            session.put(format!("terra/rover/{id}/pose"), serde_json::json!({"x":id as f64*2.,"y":0.,"yaw":0.,"heading":0.}).to_string()).wait().unwrap();
            if tick % 5 == 0 {
                session.put(format!("terra/rover/{id}/map/occupancy"), serde_json::json!({
                    "schema_version":1,"rover_id":id,"run_id":"fixture","sequence":tick,
                    "width":200,"height":200,"resolution":0.1,"origin_x":-10.,"origin_y":-10.,
                    "occupancy":vec![0;40000]
                }).to_string()).wait().unwrap();
            }
            session.put(format!("terra/rover/{id}/hardware/status"), r#"{"ready":true,"armed":true,"arming":false,"reason":"Fixture"}"#).wait().unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
