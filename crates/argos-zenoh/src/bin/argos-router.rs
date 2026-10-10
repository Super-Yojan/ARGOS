use argos_zenoh::router::Router;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut port = 7448;
    let mut address = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => port = args.next().ok_or("Missing port")?.parse()?,
            "--bind" | "--tailscale-address" => address = args.next().ok_or("Missing address")?,
            "--help" => {
                println!(
                    "argos-router [--port 7448] [--bind 0.0.0.0 | --tailscale-address 100.x.y.z]"
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
    let router = Router::default();
    router.start(port, &address)?;
    for line in router.logs() {
        println!("{line}");
    }
    while router.running() {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Ok(())
}
