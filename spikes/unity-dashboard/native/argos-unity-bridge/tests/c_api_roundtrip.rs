//! Publisher puts real ARGOS topics. The Unity C API reads them back.
use argos_unity_bridge::{
    argos_bridge_close, argos_bridge_connected, argos_bridge_last_error, argos_bridge_link,
    argos_bridge_open, argos_bridge_poll, c_string, rover_layout, serve,
};
use std::ffi::c_char;
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

static LOCK: Mutex<()> = Mutex::new(());

fn wait_for(mut predicate: impl FnMut() -> bool) {
    let start = Instant::now();
    while !predicate() {
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "bridge did not receive the publisher fleet"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn text(buf: &[u8], len: i32) -> &str {
    assert!(len >= 0, "c api returned {len}");
    std::str::from_utf8(&buf[..len as usize]).unwrap()
}

#[test]
fn publisher_fleet_arrives_through_the_c_api() {
    let _guard = LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("tcp/127.0.0.1:{}", socket.local_addr().unwrap().port());
    drop(socket);

    let _publisher = serve(&endpoint, "terra/rover", 4).expect("publisher");
    let endpoint_c = c_string(&endpoint);
    let prefix_c = c_string("terra/rover");
    let opened = argos_bridge_open(endpoint_c.as_ptr(), prefix_c.as_ptr());
    assert_eq!(opened, 0, "open failed: {}", error_text());
    assert_eq!(argos_bridge_connected(), 1);

    let mut buf = vec![0u8; 65_536];
    let mut fleet = String::new();
    wait_for(|| {
        let n = argos_bridge_poll(buf.as_mut_ptr().cast::<c_char>(), buf.len());
        if n < 0 {
            return false;
        }
        fleet = text(&buf, n).to_string();
        let parsed: serde_json::Value = serde_json::from_str(&fleet).unwrap();
        parsed["link"] == "healthy"
            && parsed["rovers"].as_array().is_some_and(|rows| {
                rows.len() == 4
                    && rows[1]["status"] == "attention"
                    && rows[2]["status"] == "critical"
            })
    });

    let mut link_buf = [0u8; 64];
    let link_len = argos_bridge_link(link_buf.as_mut_ptr().cast::<c_char>(), link_buf.len());
    assert_eq!(text(&link_buf, link_len), "healthy");

    let parsed: serde_json::Value = serde_json::from_str(&fleet).unwrap();
    let rovers = parsed["rovers"].as_array().unwrap();
    let (x, y, _) = rover_layout(1);
    assert_eq!(rovers[0]["id"], "terra-01");
    assert!((rovers[0]["position"]["x"].as_f64().unwrap() - x).abs() < 1e-6);
    assert!((rovers[0]["position"]["z"].as_f64().unwrap() - y).abs() < 1e-6);
    assert_eq!(rovers[0]["position"]["y"], 0.0);
    assert!((rovers[0]["headingDegrees"].as_f64().unwrap() - 90.0).abs() < 1e-6);
    assert_eq!(rovers[0]["status"], "normal");
    assert_eq!(rovers[1]["id"], "terra-02");
    assert_eq!(rovers[1]["status"], "attention");
    assert_eq!(rovers[2]["id"], "terra-03");
    assert_eq!(rovers[2]["status"], "critical");
    assert_eq!(rovers[3]["id"], "terra-04");
    assert_eq!(rovers[3]["status"], "normal");
    for rover in rovers {
        let age = parsed["snapshotUnixSeconds"].as_f64().unwrap()
            - rover["lastUpdateUnixSeconds"].as_f64().unwrap();
        assert!(age < 2.5, "fresh publisher sample looked stale: {age}");
    }

    argos_bridge_close();
    assert_eq!(argos_bridge_connected(), 0);
    assert_eq!(
        argos_bridge_poll(buf.as_mut_ptr().cast::<c_char>(), buf.len()),
        -1
    );
}

#[test]
fn rejected_endpoint_is_reported_without_opening() {
    let _guard = LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    argos_bridge_close();
    let endpoint = c_string("udp/127.0.0.1:1");
    let prefix = c_string("terra/rover");
    assert_eq!(argos_bridge_open(endpoint.as_ptr(), prefix.as_ptr()), -3);
    assert_eq!(argos_bridge_connected(), 0);
    let message = error_text();
    assert!(
        message.contains("tcp"),
        "last error should name the tcp requirement, got {message}"
    );
}

fn error_text() -> String {
    let mut buf = [0u8; 512];
    let n = argos_bridge_last_error(buf.as_mut_ptr().cast::<c_char>(), buf.len());
    assert!(n >= 0);
    std::str::from_utf8(&buf[..n as usize]).unwrap().to_string()
}
