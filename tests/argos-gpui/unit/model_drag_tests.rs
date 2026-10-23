use super::*;
use argos_core::PlanarPoint;
fn drag(space: DragSpace) -> GoalDrag {
    GoalDrag {
        id: 7,
        anchor: DragAnchor::Rover,
        space,
        rover: PlanarPoint { x: 0., y: 0. },
        cursor: PlanarPoint { x: 2., y: 3. },
        arrival_radius_m: None,
    }
}
#[test]
fn local_pickup_moves_only_picked_model_and_restores_after_release() {
    let frame = LocalFrame::origin(38., -77.).unwrap();
    let drag = drag(DragSpace::Local);
    let live = (38., -77.);
    assert_eq!(
        model_position(7, live, Some(frame), Some(&drag)),
        frame.geographic(2., 3.).unwrap()
    );
    assert_eq!(model_position(8, live, Some(frame), Some(&drag)), live);
    assert_eq!(model_position(7, live, Some(frame), None), live);
    assert_eq!(model_position(7, live, None, Some(&drag)), live);
}
#[test]
fn geographic_pickup_uses_cursor_without_relocating_goal_anchor() {
    let mut drag = drag(DragSpace::Geographic);
    let live = (38., -77.);
    assert_eq!(model_position(7, live, None, Some(&drag)), (2., 3.));
    drag.anchor = DragAnchor::Goal;
    assert_eq!(model_position(7, live, None, Some(&drag)), live);
}
