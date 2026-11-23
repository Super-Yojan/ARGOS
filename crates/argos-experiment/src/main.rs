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
    // Both `ring` (webrtc/zenoh) and `aws-lc-rs` (rig/reqwest) are in the
    // dependency graph, so rustls 0.23 requires choosing one explicitly.
    let _ = rustls::crypto::ring::default_provider().install_default();

    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        // The console is designed dark; don't follow the system setting.
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
