//! Keep the operator's rover selection, and that rover's map, through a short
//! reporting dropout.
//!
//! Terra builds and keeps the occupancy map. ARGOS only subscribes to the
//! selected rover's grid, so dropping the selection when a rover briefly leaves
//! the fleet (a stall or a controller restart) also dropped its map. Until the
//! operator picks something else, a lost rover stays focused for
//! [`SELECTION_HOLD_SECONDS`] and is selected again when it comes back.

/// How long a rover that vanished from the fleet keeps its selection.
pub const SELECTION_HOLD_SECONDS: f64 = 60.;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SelectionHold {
    lost: Option<(u64, f64)>,
}

impl SelectionHold {
    /// The selected rover is missing from the fleet at `now`.
    pub fn lose(&mut self, id: u64, now: f64) {
        self.lost = Some((id, now));
    }

    /// The operator picked a rover or released control; forget the hold.
    pub fn clear(&mut self) {
        self.lost = None;
    }

    /// The rover waiting to come back, if its hold has not expired.
    pub fn waiting(&self, now: f64) -> Option<u64> {
        self.lost
            .filter(|(_, since)| now - since < SELECTION_HOLD_SECONDS)
            .map(|(id, _)| id)
    }

    /// Call each refresh with nothing selected. Returns the rover to select
    /// again when it is back in `present`, and drops an expired hold.
    pub fn restore(&mut self, now: f64, present: impl Fn(u64) -> bool) -> Option<u64> {
        let Some(id) = self.waiting(now) else {
            self.lost = None;
            return None;
        };
        if present(id) {
            self.lost = None;
            Some(id)
        } else {
            None
        }
    }

    /// Rovers whose heavy topics (grid, cloud) stay subscribed.
    pub fn focus(&self, selected: Option<u64>, now: f64) -> Option<u64> {
        selected.or_else(|| self.waiting(now))
    }
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/selection_tests.rs"]
mod tests;
