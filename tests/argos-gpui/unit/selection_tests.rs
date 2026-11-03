use super::*;

#[test]
fn lost_rover_stays_focused_and_is_reselected_on_return() {
    let mut hold = SelectionHold::default();
    hold.lose(0, 10.);
    assert_eq!(hold.focus(None, 11.), Some(0));
    assert_eq!(hold.restore(11., |_| false), None);
    assert_eq!(hold.restore(12., |id| id == 0), Some(0));
    assert_eq!(hold.focus(None, 12.), None);
}

#[test]
fn hold_expires() {
    let mut hold = SelectionHold::default();
    hold.lose(3, 0.);
    let late = SELECTION_HOLD_SECONDS + 1.;
    assert_eq!(hold.focus(None, late), None);
    assert_eq!(hold.restore(late, |_| true), None);
    assert_eq!(hold.restore(late + 1., |_| true), None);
}

#[test]
fn operator_choice_wins() {
    let mut hold = SelectionHold::default();
    hold.lose(1, 0.);
    assert_eq!(hold.focus(Some(2), 1.), Some(2));
    hold.clear();
    assert_eq!(hold.restore(2., |_| true), None);
}
