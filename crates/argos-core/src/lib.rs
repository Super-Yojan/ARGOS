//! Transport-independent fleet state and operator command policy.
//!
//! `contract` defines topic and message rules; `state` tracks fleet membership,
//! goals, and freshness; `operator` tracks authority and ordered held input.
//! `telemetry`, `occupancy`, and `search` decode their respective reports. Keep
//! networking and native UI bindings in `argos-zenoh` and `argos-ffi`.

mod contract;
mod state;
mod telemetry;
pub use contract::*;
pub use state::*;
pub use telemetry::*;
mod operator;
pub use operator::*;
mod occupancy;
pub use occupancy::*;

mod autonomy;
pub use autonomy::*;

mod drive;
pub use drive::*;

mod drag;
pub use drag::*;

mod outdoor;
pub use outdoor::*;

mod search;
pub use search::*;

mod hardware;
pub use hardware::*;

pub use terra_types::connectivity_policy::{ConnectivityPolicySettings, RouterLossPolicy};
