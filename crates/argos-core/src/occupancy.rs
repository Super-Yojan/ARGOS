use crate::{Result, invalid};
use serde::{Deserialize, Serialize};
/// Observed world XY cells; row-major, +X columns and +Y rows.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OccupancyGrid {
    pub schema_version: u32,
    pub rover_id: u64,
    pub run_id: String,
    pub sequence: u64,
    pub width: u32,
    pub height: u32,
    pub resolution: f64,
    pub origin_x: f64,
    pub origin_y: f64,
    pub occupancy: Vec<i8>,
}
pub fn decode_occupancy(bytes: &[u8]) -> Result<OccupancyGrid> {
    if bytes.len() > 2_000_000 {
        return Err(invalid("oversized occupancy grid"));
    }
    let g: OccupancyGrid =
        serde_json::from_slice(bytes).map_err(|_| invalid("invalid occupancy JSON"))?;
    let cells = u64::from(g.width) * u64::from(g.height);
    if g.schema_version != 1
        || g.run_id.is_empty()
        || g.run_id.len() > 256
        || cells == 0
        || cells > 250_000
        || cells != g.occupancy.len() as u64
        || !g.resolution.is_finite()
        || g.resolution <= 0.
        || !g.origin_x.is_finite()
        || !g.origin_y.is_finite()
        || !(g.origin_x + f64::from(g.width) * g.resolution).is_finite()
        || !(g.origin_y + f64::from(g.height) * g.resolution).is_finite()
        || g.occupancy.iter().any(|v| !(-1..=100).contains(v))
    {
        return Err(invalid("invalid occupancy geometry or cells"));
    }
    Ok(g)
}
