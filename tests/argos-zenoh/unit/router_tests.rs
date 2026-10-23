use super::*;
#[test]
fn bind_policy() {
    assert_eq!(listen_addresses(7448, "").unwrap().len(), 1);
    assert_eq!(listen_addresses(7448, "100.100.1.2").unwrap().len(), 2);
    for ip in ["192.168.1.4", "example.com", "100.128.0.1"] {
        assert!(listen_addresses(7448, ip).is_err());
    }
    assert!(listen_addresses(0, "").is_err());
    assert_eq!(
        listen_addresses(7448, "0.0.0.0").unwrap(),
        vec!["0.0.0.0:7448".parse().unwrap()]
    );
}
#[test]
fn occupied_listener_survives() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let router = Router::default();
    assert!(
        router
            .start(listener.local_addr().unwrap().port(), "")
            .is_err()
    );
    router.stop().unwrap();
    assert!(listener.local_addr().is_ok());
    assert!(!router.running());
}
#[test]
fn lifecycle() {
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let router = Router::default();
    router.start(port, "").unwrap();
    assert!(router.running());
    assert!(router.start(port, "").is_err());
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
    router.stop().unwrap();
    assert!(!router.running());
    router.start(port, "").unwrap();
    router.stop().unwrap();
}
