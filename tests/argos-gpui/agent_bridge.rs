use argos_gpui::{AGENT_BRIDGE_FILE, AgentBridge};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn ask(stream: &mut TcpStream, reader: &mut BufReader<TcpStream>, body: Value) -> Value {
    stream.write_all(format!("{body}\n").as_bytes()).unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn bridge_checks_the_token_and_relays_to_the_window() {
    let dir = std::env::temp_dir().join(format!("argos-bridge-{}", std::process::id()));
    let bridge = AgentBridge::start(&dir, 0).unwrap();
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(AGENT_BRIDGE_FILE)).unwrap())
            .unwrap();
    assert_eq!(file["port"], bridge.port());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join(AGENT_BRIDGE_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "bridge file must be private");
    }
    let token = file["token"].as_str().unwrap().to_string();
    let mut stream = TcpStream::connect(("127.0.0.1", bridge.port())).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());

    let refused = ask(
        &mut stream,
        &mut reader,
        json!({"id": 1, "token": "nope", "op": "state"}),
    );
    assert_eq!(refused["ok"], false);
    assert!(refused["error"].as_str().unwrap().contains("token"));
    assert!(bridge.take_calls().is_empty());

    // Answer from "the window" on another thread while the client waits.
    let window = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(call) = bridge.take_calls().into_iter().next() {
                assert!(call.envelope.request.is_ok());
                call.answer(Ok(json!({"rovers": []})));
                return bridge;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("no call reached the window");
    });
    let reply = ask(
        &mut stream,
        &mut reader,
        json!({"id": 2, "token": token, "op": "state"}),
    );
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["id"], 2);
    assert_eq!(reply["rovers"], json!([]));
    let bridge = window.join().unwrap();
    drop(bridge);
    assert!(
        !dir.join(AGENT_BRIDGE_FILE).exists(),
        "the file goes with the bridge"
    );
}
