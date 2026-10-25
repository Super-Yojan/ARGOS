use super::*;

#[test]
fn packaged_asset_finds_assets_beside_the_executable() {
    let root = std::env::temp_dir().join(format!("argos-install-{}", std::process::id()));
    let assets = root.join("folder/assets");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(assets.join("robot.glb"), b"glb").unwrap();
    let exe = root.join("folder/argos");
    assert_eq!(
        packaged_asset(Some(&exe), "robot.glb"),
        Some(assets.join("robot.glb"))
    );
    assert_eq!(packaged_asset(Some(&exe), "missing.glb"), None);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn packaged_asset_finds_mac_bundle_resources() {
    let root = std::env::temp_dir().join(format!("argos-bundle-{}", std::process::id()));
    let contents = root.join("ARGOS.app/Contents");
    std::fs::create_dir_all(contents.join("MacOS")).unwrap();
    std::fs::create_dir_all(contents.join("Resources/assets")).unwrap();
    std::fs::write(contents.join("Resources/assets/robot.glb"), b"glb").unwrap();
    let found = packaged_asset(Some(&contents.join("MacOS/argos")), "robot.glb").unwrap();
    assert!(found.is_file());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn env_dirs_put_the_install_folder_above_the_user_config() {
    let config = PathBuf::from("/home/op/.config/argos");
    let dirs = install_env_dirs(Some(Path::new("/opt/argos/argos")), Some(config.clone()));
    assert_eq!(dirs, vec![config, PathBuf::from("/opt/argos")]);
}

#[test]
fn env_dirs_look_beside_the_mac_app_not_inside_it() {
    let dirs = install_env_dirs(
        Some(Path::new("/Applications/ARGOS.app/Contents/MacOS/argos")),
        None,
    );
    assert_eq!(dirs, vec![PathBuf::from("/Applications")]);
}
