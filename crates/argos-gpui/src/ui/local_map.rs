//! Occupancy cell colors and dotted strokes for the main map overlay.
use super::palette;
use argos_core::CellClass;
use gpui_kit::*;

pub(super) fn cell_color(class: CellClass) -> u32 {
    match class {
        CellClass::Unknown => palette::MAP_UNKNOWN,
        CellClass::Free => palette::MAP_FREE,
        CellClass::Occupied => palette::MAP_OBSTACLE,
    }
}

pub(super) fn stroke(
    window: &mut Window,
    from: Point<Pixels>,
    to: Point<Pixels>,
    color: Rgba,
    dashed: bool,
) {
    let dx = f32::from(to.x - from.x);
    let dy = f32::from(to.y - from.y);
    let len = dx.hypot(dy);
    if len < 1. {
        return;
    }
    let steps = (len / 5.).max(1.) as i32;
    for step in 0..steps {
        if dashed && step % 2 == 0 {
            continue;
        }
        let t = step as f32 / steps as f32;
        let at = point(from.x + px(dx * t), from.y + px(dy * t));
        // A dark halo keeps the line readable on bright imagery.
        let mut halo = rgb(palette::MAP_INK);
        halo.a = 0.55;
        window.paint_quad(quad(
            Bounds {
                origin: point(at.x - px(2.5), at.y - px(2.5)),
                size: size(px(5.), px(5.)),
            },
            Corners::all(px(2.5)),
            halo,
            Edges::default(),
            transparent_black(),
            BorderStyle::default(),
        ));
        window.paint_quad(quad(
            Bounds {
                origin: point(at.x - px(1.5), at.y - px(1.5)),
                size: size(px(3.), px(3.)),
            },
            Corners::default(),
            color,
            Edges::default(),
            transparent_black(),
            BorderStyle::default(),
        ));
    }
    if dashed {
        let at = point(from.x + px(dx * 0.72), from.y + px(dy * 0.72));
        window.paint_quad(quad(
            Bounds {
                origin: point(at.x - px(4.), at.y - px(4.)),
                size: size(px(8.), px(8.)),
            },
            Corners::all(px(4.)),
            color,
            Edges::all(px(1.)),
            rgb(palette::MAP_INK),
            BorderStyle::default(),
        ));
    }
}
