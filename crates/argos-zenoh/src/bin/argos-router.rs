use argos_zenoh::router::Router;
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    zenoh::init_log_from_env_or("info");
    terra_observability::initialize_runtime("router");
    let mut port = 7448;
    let mut address = String::new();
    let mut config_file = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config_file = Some(args.next().ok_or("Missing config path")?),
            "--port" => port = args.next().ok_or("Missing port")?.parse()?,
            "--bind" | "--tailscale-address" => address = args.next().ok_or("Missing address")?,
            "--help" => {
                println!(
                    "argos-router [--config /path/to/zenoh.json5] [--port 7448] [--bind 0.0.0.0 | --tailscale-address 100.x.y.z]"
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
    if let Some(path) = config_file {
        use zenoh::Wait;
        let session = zenoh::open(zenoh::Config::from_file(path)?).wait()?;
        while !session.is_closed() {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        return Ok(());
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
