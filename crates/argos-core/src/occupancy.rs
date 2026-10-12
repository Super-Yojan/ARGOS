//! Decode and validate observed occupancy grids before caching or rendering them.
//! Unknown cells retain -1; they must not be presented as observed free space.

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
    let grid: OccupancyGrid =
        serde_json::from_slice(bytes).map_err(|_| invalid("invalid occupancy JSON"))?;
    let cells = u64::from(grid.width) * u64::from(grid.height);
    if grid.schema_version != 1
        || grid.run_id.is_empty()
        || grid.run_id.len() > 256
        || cells == 0
        || cells > 250_000
        || cells != grid.occupancy.len() as u64
        || !grid.resolution.is_finite()
        || grid.resolution <= 0.
        || !grid.origin_x.is_finite()
        || !grid.origin_y.is_finite()
        || !(grid.origin_x + f64::from(grid.width) * grid.resolution).is_finite()
        || !(grid.origin_y + f64::from(grid.height) * grid.resolution).is_finite()
        || grid.occupancy.iter().any(|v| !(-1..=100).contains(v))
    {
        return Err(invalid("invalid occupancy geometry or cells"));
    }
    Ok(grid)
}
