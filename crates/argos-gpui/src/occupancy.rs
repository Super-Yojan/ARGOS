//! Lossless horizontal spans for occupancy painting; never merge across unknown cells.
use argos_core::{CellClass, LocalMap};

/// (class, first cell index, last cell index), inclusive, within a single row.
/// GPUI inserts every painted path into its scene bounds tree. Painting one
/// path per cell overwhelms that tree with overlapping multi-rover overlays.
/// Row spans retain the original cell boundaries without downsampling.
pub fn occupancy_runs(grid: &LocalMap) -> Vec<(CellClass, usize, usize)> {
    let mut runs = Vec::new();
    if grid.cols == 0 {
        return runs;
    }
    for (row, cells) in grid.cells.chunks(grid.cols as usize).enumerate() {
        let mut start = 0;
        while start < cells.len() {
            let class = cells[start];
            let mut end = start + 1;
            while end < cells.len() && cells[end] == class {
                end += 1;
            }
            if class != CellClass::Unknown {
                let offset = row * grid.cols as usize;
                runs.push((class, offset + start, offset + end - 1));
            }
            start = end;
        }
    }
    runs
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/occupancy_tests.rs"]
mod tests;
