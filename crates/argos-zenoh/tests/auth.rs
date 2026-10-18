use zenoh::Wait;

#[test]
fn password_router_rejects_missing_and_wrong_credentials() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let path = std::env::temp_dir().join(format!("argos-auth-{}", uuid::Uuid::new_v4()));
    std::fs::write(&path, "operator:test-only-password\n").unwrap();
    let mut config = zenoh::Config::default();
    config.insert_json5("mode", "\"router\"").unwrap();
    config
        .insert_json5(
            "listen/endpoints",
            &serde_json::json!([format!("tcp/127.0.0.1:{port}")]).to_string(),
        )
        .unwrap();
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    config
        .insert_json5(
            "transport/auth/usrpwd/dictionary_file",
            &serde_json::json!(path).to_string(),
        )
        .unwrap();
    let router = zenoh::open(config).wait().unwrap();
    let endpoint = format!("tcp/127.0.0.1:{port}");
    assert!(argos_zenoh::Client::connect(&endpoint, "auth-test").is_err());
    assert!(
        argos_zenoh::Client::connect_authenticated(
            &endpoint,
            "auth-test",
            Some(("operator", "wrong"))
        )
        .is_err()
    );
    let client = argos_zenoh::Client::connect_authenticated(
        &endpoint,
        "auth-test",
        Some(("operator", "test-only-password")),
    )
    .unwrap();
    client.disconnect();
    router.close().wait().unwrap();
    std::fs::remove_file(path).unwrap();
}
