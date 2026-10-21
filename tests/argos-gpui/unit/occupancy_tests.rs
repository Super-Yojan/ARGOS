use super::*;
fn grid(cols: u32, cells: Vec<CellClass>) -> LocalMap {
    LocalMap {
        cols,
        rows: cells.len() as u32 / cols,
        cells,
        centers: vec![],
        origin_x: -10.,
        origin_y: -10.,
        span_x: 20.,
        span_y: 20.,
    }
}
#[test]
fn two_full_rovers_need_only_four_hundred_spans() {
    let map = grid(200, vec![CellClass::Free; 40_000]);
    assert_eq!(occupancy_runs(&map).len() * 2, 400);
}
#[test]
fn spans_preserve_every_class_gap_and_row_boundary() {
    // Exhaust all small three-class maps, including isolated occupied cells.
    for mut pattern in 0..3usize.pow(8) {
        let mut cells = Vec::new();
        for _ in 0..8 {
            cells.push([CellClass::Unknown, CellClass::Free, CellClass::Occupied][pattern % 3]);
            pattern /= 3;
        }
        let map = grid(4, cells.clone());
        let mut restored = vec![CellClass::Unknown; 8];
        for (class, start, end) in occupancy_runs(&map) {
            assert_eq!(start / 4, end / 4);
            assert!(start <= end);
            for cell in &mut restored[start..=end] {
                assert_eq!(*cell, CellClass::Unknown);
                *cell = class;
            }
        }
        assert_eq!(restored, cells);
    }
}
