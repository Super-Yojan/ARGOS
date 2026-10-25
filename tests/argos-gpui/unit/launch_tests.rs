use super::*;
fn parse(args: &[&str]) -> Result<Option<LaunchMode>, String> {
    LaunchMode::parse(args.iter().map(|s| s.to_string()))
}
#[test]
fn selects_mode_and_rejects_mistakes() {
    assert_eq!(parse(&[]), Ok(Some(LaunchMode::Real)));
    assert_eq!(parse(&["--sim"]), Ok(Some(LaunchMode::Sim)));
    assert_eq!(parse(&["--real"]), Ok(Some(LaunchMode::Real)));
    assert!(parse(&["--sim", "--real"]).is_err());
    assert_eq!(parse(&["--mode=real"]), Ok(Some(LaunchMode::Real)));
    assert_eq!(parse(&["--help"]), Ok(None));
    for args in [
        vec!["--mode"],
        vec!["--mode", "oops"],
        vec!["--mod", "sim"],
        vec!["--mode=sim", "--mode=real"],
    ] {
        assert!(parse(&args).is_err());
    }
}
#[test]
fn sim_clears_hosted_auth_but_preserves_maps_and_real_preserves_settings() {
    let mut settings = BTreeMap::from([
        ("ARGOS_ZENOH_PASSWORD".into(), "test-only".into()),
        ("GOOGLE_MAPS_API_KEY".into(), "test-map-key".into()),
    ]);
    let original = settings.clone();
    LaunchMode::Real.apply(&mut settings);
    assert_eq!(settings, original);
    LaunchMode::Sim.apply(&mut settings);
    assert_eq!(settings["ARGOS_ZENOH_PASSWORD"], "");
    assert_eq!(settings["ARGOS_ZENOH_ENDPOINT"], "tcp/127.0.0.1:7447");
    assert_eq!(settings["ARGOS_ZENOH_PREFIX"], "terra/rover");
    assert_eq!(settings["GOOGLE_MAPS_API_KEY"], "test-map-key");
}
#[test]
fn each_mode_names_its_window() {
    assert_eq!(LaunchMode::Sim.title(), "ARGOS · Simulation");
    assert_eq!(LaunchMode::Real.title(), "ARGOS · Real fleet");
    assert_ne!(LaunchMode::Sim.label(), LaunchMode::Real.label());
}
