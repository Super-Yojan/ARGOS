//! ARGOS operator console.
//!
//! - [`fleet`]: the one connection to the rovers; reads their state, sends
//!   commands.
//! - [`components`]: the reusable parts of the screen.
//! - [`pages`]: Supervise and Drive, each an arrangement of components.
//! - [`console`]: the shell that switches between pages.
//! - [`theme`]: colours and fonts.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

mod components;
mod console;
mod fleet;
mod fpv;
mod llm;
mod pages;
mod snapshot;
mod theme;

use console::Console;

fn main() {
    load_dotenv();

    // Both `ring` (webrtc/zenoh) and `aws-lc-rs` (rig/reqwest) are in the
    // dependency graph, so rustls 0.23 requires choosing one explicitly.
    let _ = rustls::crypto::ring::default_provider().install_default();

    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        // Neutral carbon-dark operations console palette.
        Theme::change(ThemeMode::Dark, None, cx);

        let window_size = size(px(1440.0), px(900.0));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, window_size, cx))),
            ..Default::default()
        };
        open_window(options, cx, |window, cx| cx.new(|cx| Console::new(window, cx)))
            .expect("Failed to open window");
    });
}

/// Load key=value pairs from `.env` (current directory or workspace root) into
/// the process environment without overwriting already-exported variables.
fn load_dotenv() {
    let candidates = [
        std::path::PathBuf::from(".env"),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env"),
    ];
    for path in candidates {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        for raw_line in contents.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line).trim();
            let Some((key, val)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            if key.is_empty() || std::env::var_os(key).is_some() {
                continue;
            }
            let val = val.trim();
            let val = val
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| val.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(val);
            // SAFETY: called at startup on the main thread before any worker threads spawn.
            unsafe { std::env::set_var(key, val) };
        }
        break;
    }
}
