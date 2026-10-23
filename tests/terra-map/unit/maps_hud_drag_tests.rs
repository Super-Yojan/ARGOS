use super::previous_map_drag;
use gpui_kit::{point, px};
#[test]
fn rover_press_intercepted_before_map_never_starts_camera_drag() {
    let mut last = None;
    for step in 0..4 {
        assert!(previous_map_drag(&mut last, point(px(step as f32), px(2.)), true).is_none());
    }
    assert!(last.is_none());
}
#[test]
fn map_press_can_move_until_release_without_reacquiring() {
    let start = point(px(10.), px(20.));
    let next = point(px(15.), px(30.));
    let mut last = Some(start);
    assert_eq!(previous_map_drag(&mut last, next, true), Some(start));
    assert_eq!(last, Some(next));
    assert_eq!(previous_map_drag(&mut last, next, false), None);
    assert_eq!(previous_map_drag(&mut last, start, true), None);
}
