//! The console's reusable parts. Each file is one component.
//!
//! Two kinds:
//!
//! - **Views with their own state** are GPUI entities (`cx.new(..)`):
//!   [`fleet_map::FleetMap`] (pan, zoom, goal picking),
//!   [`scene_view::SceneView`] (3D camera, mesh),
//!   [`drive_controls::DriveControls`] (keys, gamepad, the 20 Hz command
//!   loop) and [`chat::AgentChat`].
//! - **Views without state** are built fresh on every render from the data
//!   they show: [`top_bar::TopBar`], [`help_queue::HelpQueue`],
//!   [`rover_list::RoverList`], [`focus_panel::FocusPanel`],
//!   [`level_picker::LevelPicker`]. They report
//!   clicks through an [`ActionHandler`].
//!
//! The only thing they share is the [`crate::fleet::Fleet`] handle.

pub mod chat;
pub mod drive_controls;
pub mod fleet_map;
pub mod focus_panel;
pub mod fpv_view;
pub mod help_queue;
pub mod level_picker;
pub mod mesh_renderer;
pub mod rover_list;
pub mod scene_view;
pub mod top_bar;
pub mod widgets;

use std::rc::Rc;

use gpui_kit::*;

use crate::fleet::{Request, Rover, RoverAction};
use widgets::{ButtonLook, button};

/// Called when the operator picks an action for a rover. Build one with
/// `action_handler(cx.listener(|this, action, window, cx| ..))`.
pub type ActionHandler = Rc<dyn Fn(&Request, &mut Window, &mut App)>;

pub fn action_handler(listener: impl Fn(&Request, &mut Window, &mut App) + 'static) -> ActionHandler {
    Rc::new(listener)
}

/// A button that reports `action` for `rover` when clicked. It is greyed
/// out while the rover would refuse the action.
pub fn action_button(
    rover: &Rover,
    action: RoverAction,
    label: impl Into<SharedString>,
    look: ButtonLook,
    on_action: &ActionHandler,
) -> Stateful<Div> {
    let id = SharedString::from(format!("{action:?}-{}", rover.id));
    if !rover.allows(&action) {
        return button(id, label, ButtonLook::Disabled);
    }

    let on_action = on_action.clone();
    let rover_action = Request { rover: rover.id, action };
    button(id, label, look).on_click(move |_, window, cx| {
        // Don't also trigger the card or row the button sits on.
        cx.stop_propagation();
        on_action(&rover_action, window, cx);
    })
}
