use argos_core::{ROVER_QUIET_SECONDS, ROVER_RECOVER_SECONDS, Reporting, ReportingHold};

fn quiet(silent_s: f64) -> Reporting {
    Reporting::NotReporting { silent_s }
}

#[test]
fn a_short_gap_stays_fresh() {
    let mut hold = ReportingHold::default();
    assert_eq!(hold.observe(Reporting::Fresh, 0.), Reporting::Fresh);
    // Raw reading flips at 2.5 s; the hold waits for the quiet delay.
    assert_eq!(hold.observe(quiet(2.6), 2.6), Reporting::Fresh);
    assert_eq!(hold.observe(quiet(4.9), 4.9), Reporting::Fresh);
    assert_eq!(hold.observe(Reporting::Fresh, 5.), Reporting::Fresh);
}

#[test]
fn a_long_gap_shows_and_needs_steady_data_to_clear() {
    let mut hold = ReportingHold::default();
    hold.observe(Reporting::Fresh, 0.);
    let shown = hold.observe(quiet(ROVER_QUIET_SECONDS), 5.);
    assert_eq!(shown, quiet(ROVER_QUIET_SECONDS));
    // One burst of data does not flash it back to fresh; the timer freezes.
    assert_eq!(
        hold.observe(Reporting::Fresh, 6.),
        quiet(ROVER_QUIET_SECONDS)
    );
    assert_eq!(
        hold.observe(Reporting::Fresh, 7.),
        quiet(ROVER_QUIET_SECONDS)
    );
    // Quiet again before the recover time: stays quiet, timer restarts.
    assert_eq!(hold.observe(quiet(2.6), 9.6), quiet(2.6));
    // Steady data for the recover time clears it.
    hold.observe(Reporting::Fresh, 10.);
    assert_eq!(
        hold.observe(Reporting::Fresh, 10. + ROVER_RECOVER_SECONDS - 0.1),
        quiet(2.6)
    );
    assert_eq!(
        hold.observe(Reporting::Fresh, 10. + ROVER_RECOVER_SECONDS),
        Reporting::Fresh
    );
}

#[test]
fn link_lost_still_escalates() {
    let mut hold = ReportingHold::default();
    hold.observe(quiet(6.), 6.);
    let lost = Reporting::LinkLost { silent_s: 15. };
    assert_eq!(hold.observe(lost, 15.), lost);
}

#[test]
fn labels_no_longer_claim_the_rover_holds() {
    assert_eq!(quiet(12.3).label().unwrap(), "Not reporting · 12 s");
    assert!(!quiet(3.).next_step().unwrap().contains("holds still"));
}
