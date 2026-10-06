use argos_ffi::*;
#[test]
fn disconnected_client_is_safe_and_rejects_actions() {
    let client = ArgosClient::new();
    assert_eq!(client.snapshot().link, "disconnected");
    client.disconnect();
    client.disconnect();
    assert!(
        client
            .send_goal(
                0,
                Waypoint::Local {
                    x: 1.,
                    y: 0.,
                    yaw: None
                }
            )
            .is_err()
    );
    assert!(
        client
            .connect(ConnectionConfig {
                endpoint: "invalid".into(),
                prefix: "terra/rover".into()
            })
            .is_err()
    );
    assert_eq!(client.snapshot().link, "disconnected");
}
