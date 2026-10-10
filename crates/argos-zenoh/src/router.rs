//! A router owned by the caller. Existing listeners are never adopted or stopped.
use argos_core::{Error, Result};
use std::{
    net::{Ipv4Addr, SocketAddr, TcpListener},
    sync::Mutex,
};
use zenoh::Wait;

pub struct Router {
    session: Mutex<Option<zenoh::Session>>,
    logs: Mutex<Vec<String>>,
}
impl Default for Router {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            logs: Mutex::new(Vec::new()),
        }
    }
}
impl Router {
    pub fn start(&self, port: u16, private_address: &str) -> Result<()> {
        let mut state = self.session.lock().unwrap();
        if state.is_some() {
            return Err(Error(
                "Router already running; stop it before changing its bind".into(),
            ));
        }
        let addresses = listen_addresses(port, private_address)?;
        // Probe every requested bind, then release probes immediately before Zenoh binds.
        let probes: std::io::Result<Vec<_>> =
            addresses.iter().map(|a| TcpListener::bind(a)).collect();
        let probes = probes.map_err(|e| { self.log(format!("Bind failed: {e}")); Error(format!("Router bind failed: {e}. An existing router may own this port; use Remote host to connect without managing it.")) })?;
        drop(probes);
        let endpoints: Vec<_> = addresses.iter().map(|a| format!("tcp/{a}")).collect();
        let mut config = zenoh::Config::default();
        for (key, value) in [
            ("mode", "\"router\"".to_owned()),
            (
                "listen/endpoints",
                serde_json::to_string(&endpoints).unwrap(),
            ),
            ("scouting/multicast/enabled", "false".into()),
            ("scouting/gossip/enabled", "false".into()),
        ] {
            config
                .insert_json5(key, &value)
                .map_err(|e| Error(e.to_string()))?;
        }
        let session = zenoh::open(config).wait().map_err(|e| {
            self.log(format!("Start failed: {e}"));
            Error(e.to_string())
        })?;
        self.log(format!("Listening on {}", endpoints.join(", ")));
        *state = Some(session);
        Ok(())
    }
    pub fn stop(&self) -> Result<()> {
        if let Some(session) = self.session.lock().unwrap().take() {
            session.close().wait().map_err(|e| Error(e.to_string()))?;
            self.log("Router stopped".into());
        }
        Ok(())
    }
    pub fn running(&self) -> bool {
        self.session
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| !s.is_closed())
    }
    pub fn logs(&self) -> Vec<String> {
        self.logs.lock().unwrap().clone()
    }
    fn log(&self, message: String) {
        let mut logs = self.logs.lock().unwrap();
        logs.push(message);
        if logs.len() > 100 {
            logs.remove(0);
        }
    }
}
pub fn listen_addresses(port: u16, private_address: &str) -> Result<Vec<SocketAddr>> {
    if port == 0 {
        return Err(Error("Router port must be nonzero".into()));
    }
    let mut addresses = vec![SocketAddr::from(([127, 0, 0, 1], port))];
    if !private_address.trim().is_empty() {
        let ip: Ipv4Addr = private_address
            .parse()
            .map_err(|_| Error("Enter a Tailscale IPv4 address".into()))?;
        let [a, b, _, _] = ip.octets();
        if ip.is_unspecified() {
            return Ok(vec![SocketAddr::from((ip, port))]);
        }
        if a != 100 || !(64..=127).contains(&b) {
            return Err(Error(
                "Use 0.0.0.0 for LAN sharing or a Tailscale address in 100.64.0.0/10".into(),
            ));
        }
        addresses.push(SocketAddr::from((ip, port)));
    }
    Ok(addresses)
}
#[cfg(test)]
mod tests {
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
}
