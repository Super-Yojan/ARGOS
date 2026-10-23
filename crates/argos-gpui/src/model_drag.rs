//! A dragged model is a destination preview, never a change to live telemetry.
use argos_core::{DragAnchor, DragSpace, GoalDrag, LocalFrame};

pub fn model_position(
    id: u64,
    live: (f64, f64),
    frame: Option<LocalFrame>,
    drag: Option<&GoalDrag>,
) -> (f64, f64) {
    let Some(drag) = drag.filter(|drag| drag.id == id && drag.anchor == DragAnchor::Rover) else {
        return live;
    };
    match drag.space {
        DragSpace::Geographic => (drag.cursor.x, drag.cursor.y),
        DragSpace::Local => frame
            .and_then(|frame| frame.geographic(drag.cursor.x, drag.cursor.y).ok())
            .unwrap_or(live),
    }
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/model_drag_tests.rs"]
mod tests;
