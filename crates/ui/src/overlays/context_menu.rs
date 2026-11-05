//! A right-click menu for tabs and side bar rows. Every item is a menu bar and Command
//! Palette command too, so the keyboard never needs it.

use crate::workspace::*;
use gpui::{anchored, deferred, Pixels, Point, SharedString};

pub struct ContextMenu {
    pub at: Point<Pixels>,
    /// `None` is a separator.
    pub items: Vec<Option<(SharedString, Box<dyn Action>)>>,
}

/// A menu row.
pub fn item(label: impl Into<SharedString>, a: impl Action) -> Option<(SharedString, Box<dyn Action>)> {
    Some((label.into(), Box::new(a)))
}

impl Workspace {
    pub(crate) fn open_context_menu(
        &mut self,
        at: Point<Pixels>,
        items: Vec<Option<(SharedString, Box<dyn Action>)>>,
        cx: &mut Context<Self>,
    ) {
        self.cancel_quick_panel(cx);
        self.context_menu = Some(ContextMenu { at, items });
        cx.notify();
    }

    /// The menu closes; a tab it was opened on stops being the target once its command ran.
    pub(crate) fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.context_menu.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn render_context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.context_menu.as_ref()?;
        let rows = menu.items.iter().enumerate().map(|(n, it)| match it {
            None => div().my(px(4.)).mx(px(4.)).h(px(1.)).bg(ui(theme::BORDER_VARIANT)).into_any_element(),
            Some((label, action)) => {
                let action = action.boxed_clone();
                div()
                    .id(("context-item", n))
                    .px(px(10.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .whitespace_nowrap()
                    .rounded(px(kit::RADIUS))
                    .hover(|s| s.bg(ui(theme::ELEMENT_HOVER)))
                    .cursor_pointer()
                    .child(label.clone())
                    .on_click(cx.listener(move |ws, _: &ClickEvent, window, cx| {
                        ws.context_menu = None;
                        window.dispatch_action(action.boxed_clone(), cx);
                        cx.notify();
                    }))
                    .into_any_element()
            }
        });
        let body = div()
            .id("context-menu")
            .occlude()
            .min_w(px(200.))
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(px(kit::OVERLAY_RADIUS))
            .border_1()
            .border_color(ui(theme::MENU_BORDER))
            .bg(ui(theme::OVERLAY_BG))
            .text_color(ui(theme::UI_FG))
            .text_size(px(13.))
            .shadow_lg()
            .children(rows)
            .on_mouse_down_out(cx.listener(|ws, _, _, cx| {
                ws.context_tab = None;
                ws.close_context_menu(cx);
            }));
        Some(deferred(anchored().position(menu.at).snap_to_window().child(body)).with_priority(1).into_any_element())
    }
}
