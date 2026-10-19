//! Decode and validate observed occupancy grids before caching or rendering them.
//! Unknown cells retain -1; they must not be presented as observed free space.
//!
//! [`OCCUPIED_THRESHOLD`] is the only occupied cutoff. Dashboard docs and the
//! removed browser use 65. The retired argos-grid-viewer used 50; that cutoff
//! is not used.

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

/// A cell at or above this value is occupied. Below it, a non-negative cell is free.
pub const OCCUPIED_THRESHOLD: i8 = 65;

/// Display class for one occupancy cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellClass {
    Unknown,
    Free,
    Occupied,
}

/// Classify one cell. Values below 0 are unknown, including the wire value -1.
pub fn classify_cell(value: i8) -> CellClass {
    if value < 0 {
        CellClass::Unknown
    } else if value >= OCCUPIED_THRESHOLD {
        CellClass::Occupied
    } else {
        CellClass::Free
    }
}

/// Center of cell `(col, row)` in the grid frame. Columns are +X, rows are +Y.
pub fn cell_center(grid: &OccupancyGrid, col: u32, row: u32) -> Option<(f64, f64)> {
    if col >= grid.width || row >= grid.height {
        return None;
    }
    Some((
        grid.origin_x + (f64::from(col) + 0.5) * grid.resolution,
        grid.origin_y + (f64::from(row) + 0.5) * grid.resolution,
    ))
}

/// Downsampled local map. Occupied wins over free, and free wins over unknown.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalMap {
    pub cols: u32,
    pub rows: u32,
    pub cells: Vec<CellClass>,
    /// Local-metre center of each display cell, row-major.
    pub centers: Vec<(f64, f64)>,
    pub origin_x: f64,
    pub origin_y: f64,
    pub span_x: f64,
    pub span_y: f64,
}

/// Merge the grid into at most `max_axis` cells on its longer side.
pub fn local_map(grid: &OccupancyGrid, max_axis: u32) -> LocalMap {
    let max_axis = max_axis.max(1);
    let long = grid.width.max(grid.height).max(1);
    let scale = (u64::from(long).div_ceil(u64::from(max_axis))).max(1);
    let scale_u = u32::try_from(scale).unwrap_or(u32::MAX);
    let cols = grid.width.div_ceil(scale_u).max(1);
    let rows = grid.height.div_ceil(scale_u).max(1);
    let n = (cols * rows) as usize;
    let mut cells = vec![CellClass::Unknown; n];
    let mut rank = vec![0u8; n];
    for row in 0..grid.height {
        for col in 0..grid.width {
            let index = ((row / scale_u) * cols + (col / scale_u)) as usize;
            let class = classify_cell(grid.occupancy[(row * grid.width + col) as usize]);
            let next = match class {
                CellClass::Unknown => 0,
                CellClass::Free => 1,
                CellClass::Occupied => 2,
            };
            if next >= rank[index] {
                rank[index] = next;
                cells[index] = class;
            }
        }
    }
    let mut centers = Vec::with_capacity(n);
    for row in 0..rows {
        for col in 0..cols {
            let c0 = col * scale_u;
            let r0 = row * scale_u;
            let c1 = (c0 + scale_u).min(grid.width);
            let r1 = (r0 + scale_u).min(grid.height);
            let x0 = grid.origin_x + f64::from(c0) * grid.resolution;
            let x1 = grid.origin_x + f64::from(c1) * grid.resolution;
            let y0 = grid.origin_y + f64::from(r0) * grid.resolution;
            let y1 = grid.origin_y + f64::from(r1) * grid.resolution;
            centers.push(((x0 + x1) * 0.5, (y0 + y1) * 0.5));
        }
    }
    LocalMap {
        cols,
        rows,
        cells,
        centers,
        origin_x: grid.origin_x,
        origin_y: grid.origin_y,
        span_x: f64::from(grid.width) * grid.resolution,
        span_y: f64::from(grid.height) * grid.resolution,
    }
}
