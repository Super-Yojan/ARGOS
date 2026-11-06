//! Small shared building blocks for the inspector: section titles, hints,
//! cautions, and label/value rows.

use gpui_kit::base::v_flex;
use gpui_kit::component::{
    ActiveTheme, StyledExt,
    description_list::{DescriptionItem, DescriptionList},
};
use gpui_kit::{prelude::*, *};

pub(super) fn section(title: &'static str, cx: &App) -> Div {
    v_flex().gap_2().child(
        div()
            .text_xs()
            .font_semibold()
            .text_color(cx.theme().muted_foreground)
            .child(title),
    )
}

pub(super) fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

pub(super) fn caution(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().warning)
        .child(text.into())
}

pub(super) fn rows(items: Vec<(&'static str, String)>) -> DescriptionList {
    DescriptionList::horizontal()
        .columns(1)
        .bordered(false)
        .label_width(rems(6.))
        .children(
            items
                .into_iter()
                .map(|(label, value)| DescriptionItem::new(label).value(value)),
        )
}

/// Selected styling that reads at a glance: a picked button takes the accent
/// (primary) colour instead of the kit's faint grey "active" fill.
pub(super) trait Picked {
    fn picked(self, on: bool) -> Self;
}

impl Picked for gpui_kit::component::button::Button {
    fn picked(self, on: bool) -> Self {
        use gpui_kit::component::{Selectable, button::ButtonVariants};
        let button = self.selected(on);
        if on { button.primary() } else { button }
    }
}
