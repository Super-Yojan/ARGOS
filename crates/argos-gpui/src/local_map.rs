//! Occupancy cell colors and dotted strokes for the main map overlay.
use argos_core::CellClass;
use gpui_kit::*;

pub(super) fn cell_color(class: CellClass) -> u32 {
    match class {
        CellClass::Unknown => 0x6e7c84,
        CellClass::Free => 0xd7efe0,
        CellClass::Occupied => 0x12161a,
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
            Corners::default(),
            color,
            Edges::default(),
            transparent_black(),
            BorderStyle::default(),
        ));
    }
}
