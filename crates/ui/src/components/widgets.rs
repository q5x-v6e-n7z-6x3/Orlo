//! The chrome's shared controls, finished the way Zed finishes its own: icon buttons that stay
//! quiet until hovered and name themselves in a tooltip, text buttons with a hairline edge,
//! section headings, panel headers and fields, in the theme's colours.

use crate::chrome::icons::{icon, Icon};
use crate::chrome::theme::{self, ui};
use gpui::{div, prelude::*, px, AnyView, App, Context, Div, ElementId, FontWeight, SharedString, Stateful, Window};

/// An icon button's square, and the icon in it.
pub const ICON_BUTTON: f32 = 22.;
pub const ICON_SIZE: f32 = 14.;
/// The corner of controls; overlays round theirs by the 4pt their rows are inset more.
pub const RADIUS: f32 = 6.;
pub const OVERLAY_RADIUS: f32 = 10.;

/// A square button showing `i`, quiet until the mouse is over it; `tip` names it.
pub fn icon_button(id: impl Into<ElementId>, i: Icon, tip: impl Into<SharedString>) -> Stateful<Div> {
    icon_toggle(id, i, tip, false)
}

/// `icon_button` for an option or a panel: while `on`, it keeps a fill and the full colour.
pub fn icon_toggle(id: impl Into<ElementId>, i: Icon, tip: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .group("icon-button")
        .flex_none()
        .size(px(ICON_BUTTON))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS))
        .cursor_pointer()
        .hover(|s| s.bg(ui(theme::ELEMENT_HOVER)))
        .when(on, |d| d.bg(ui(theme::BUTTON_SELECTED_BG)))
        // an svg takes no colour from its parent: it is given one, and the hover's
        .child(
            icon(i, ICON_SIZE)
                .text_color(ui(match on {
                    true => theme::ICON,
                    false => theme::ICON_MUTED,
                }))
                .group_hover("icon-button", |s| s.text_color(ui(theme::ICON))),
        )
        .tooltip(tooltip(tip.into()))
}

/// A text button: a light fill inside a hairline edge.
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(24.))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded(px(RADIUS))
        .border_1()
        .border_color(ui(theme::BORDER))
        .bg(ui(theme::BUTTON_BG))
        .text_color(ui(theme::UI_FG))
        .text_size(px(12.))
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.bg(ui(theme::ELEMENT_HOVER)))
        .child(label.into())
}

/// A text button without a fill or edge until the mouse is over it.
pub fn ghost_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(22.))
        .px(px(6.))
        .flex()
        .items_center()
        .gap(px(5.))
        .rounded(px(RADIUS))
        .text_color(ui(theme::UI_FG))
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.bg(ui(theme::ELEMENT_HOVER)))
        .child(label.into())
}

/// A section's name over its rows: small, semibold and muted, as Zed's panels head theirs.
pub fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ui(theme::SIDEBAR_HEADING))
        .whitespace_nowrap()
        .child(text.into())
}

/// A panel's title row: a 28pt strip on the panel surface, underlined by a hairline. Callers
/// add the title and the buttons.
pub fn panel_header() -> Div {
    div()
        .h(px(28.))
        .flex_shrink_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .pl(px(10.))
        .pr(px(4.))
        .bg(ui(theme::PANEL_BG))
        .border_b_1()
        .border_color(ui(theme::BORDER_VARIANT))
        .text_color(ui(theme::UI_FG))
}

/// The box around a single-line field.
pub fn field() -> Div {
    div().bg(ui(theme::WIDGET_BG)).rounded(px(RADIUS)).border_1().border_color(ui(theme::BORDER))
}

/// A tooltip naming a control, for `.tooltip(…)`.
pub fn tooltip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |_, cx| {
        let text = text.clone();
        cx.new(|_| Tooltip(text)).into()
    }
}

pub struct Tooltip(SharedString);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // clear of the pointer, which gpui puts the tooltip's corner on
        div().pl(px(6.)).pt(px(14.)).child(
            div()
                .px(px(8.))
                .py(px(4.))
                .rounded(px(RADIUS))
                .border_1()
                .border_color(ui(theme::MENU_BORDER))
                .bg(ui(theme::OVERLAY_BG))
                .shadow_md()
                .font_family(theme::UI_FONT)
                .text_size(px(12.))
                .text_color(ui(theme::UI_FG))
                .whitespace_nowrap()
                .child(self.0.clone()),
        )
    }
}
