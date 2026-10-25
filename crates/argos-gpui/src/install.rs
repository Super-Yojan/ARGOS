//! Where a downloaded ARGOS finds its files.
//!
//! A release zip puts `assets/` beside the executable, or in
//! `ARGOS.app/Contents/Resources/assets` on macOS (`scripts/package.py`).
//! A `cargo run` build keeps reading the files in the source tree.

use std::path::{Path, PathBuf};

/// The rover marker model.
pub fn robot_model() -> PathBuf {
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../terra-map/src/assets/robot.glb");
    let exe = std::env::current_exe().ok();
    packaged_asset(exe.as_deref(), "robot.glb").unwrap_or(source)
}

/// `name` in a packaged layout around `exe`, if it is there.
pub fn packaged_asset(exe: Option<&Path>, name: &str) -> Option<PathBuf> {
    let dir = exe?.parent()?;
    [
        dir.join("assets").join(name),
        dir.join("../Resources/assets").join(name),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

/// Folders a packaged build reads `.env` from, lowest priority first: the
/// per-user config folder, then the folder the executable (or `ARGOS.app`)
/// sits in.
pub fn install_env_dirs(exe: Option<&Path>, config: Option<PathBuf>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = config.into_iter().collect();
    if let Some(dir) = exe.and_then(Path::parent) {
        // ARGOS.app/Contents/MacOS/argos: the .env goes beside ARGOS.app.
        let bundle = dir
            .ends_with("Contents/MacOS")
            .then(|| dir.parent().and_then(Path::parent).and_then(Path::parent))
            .flatten();
        dirs.push(bundle.unwrap_or(dir).to_path_buf());
    }
    dirs
}

/// `~/Library/Application Support/ARGOS` on macOS, `%APPDATA%\ARGOS` on
/// Windows, and `$XDG_CONFIG_HOME/argos` (default `~/.config/argos`) on Linux.
pub fn user_config_dir() -> Option<PathBuf> {
    let var = |key: &str| {
        std::env::var_os(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Application Support/ARGOS"))
    } else if cfg!(windows) {
        var("APPDATA").map(|dir| dir.join("ARGOS"))
    } else {
        var("XDG_CONFIG_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".config")))
            .map(|dir| dir.join("argos"))
    }
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/install_tests.rs"]
mod tests;
