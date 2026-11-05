//! Pane rendering, dividers, and sidebar geometry for GPUI workspaces.

use crate::workspace::*;
pub use orlo_workspace::layout::*;

impl Workspace {
    pub(crate) fn drag_sidebar(&mut self, e: &DragMoveEvent<SidebarDrag>, window: &mut Window, cx: &mut Context<Self>) {
        let x = f32::from(e.event.position.x - e.bounds.left());
        self.sidebar.drag_to(x, f32::from(window.viewport_size().width));
        cx.notify();
    }

    pub(crate) fn drag_divider(&mut self, e: &DragMoveEvent<DividerDrag>, _: &mut Window, cx: &mut Context<Self>) {
        // the new geometry is kept once the drag settles
        autosave(cx);
        let DividerDrag(axis, i) = *e.drag(cx);
        let b = e.bounds;
        let (pos, size) = match axis {
            Axis::Cols => (e.event.position.x - b.left(), b.size.width),
            Axis::Rows => (e.event.position.y - b.top(), b.size.height),
        };
        let size = f32::from(size);
        if size > 0. {
            let min = theme::GROUP_MIN / size;
            self.views.layout.drag(axis, i, f32::from(pos) / size, min);
            cx.notify();
        }
    }

    /// A grab strip over each divider segment, where the groups on its two sides meet; the
    /// groups area turns its drag into a new fraction for that grid line.
    pub(crate) fn render_dividers(&self) -> Vec<AnyElement> {
        let layout = &self.views.layout;
        // centred on the hairline, which sits just inside the next group
        let offset = px((theme::GROUP_BORDER_W - theme::DIVIDER_HIT) / 2.);
        layout
            .dividers()
            .into_iter()
            .enumerate()
            .map(|(n, d)| {
                let strip = div()
                    .id(("divider", n))
                    .absolute()
                    .occlude()
                    .on_drag(DividerDrag(d.axis, d.index), |_, _, _, cx| cx.new(|_| EmptyView));
                let anchor = div().absolute();
                match d.axis {
                    Axis::Cols => anchor
                        .left(relative(layout.cols[d.index]))
                        .top(relative(d.from))
                        .h(relative(d.to - d.from))
                        .child(
                            strip
                                .left(offset)
                                .top_0()
                                .bottom_0()
                                .w(px(theme::DIVIDER_HIT))
                                .cursor(CursorStyle::ResizeLeftRight),
                        ),
                    Axis::Rows => anchor
                        .top(relative(layout.rows[d.index]))
                        .left(relative(d.from))
                        .w(relative(d.to - d.from))
                        .child(
                            strip
                                .top(offset)
                                .left_0()
                                .right_0()
                                .h(px(theme::DIVIDER_HIT))
                                .cursor(CursorStyle::ResizeUpDown),
                        ),
                }
                .into_any_element()
            })
            .collect()
    }

    /// Each group at its layout cell: its own tab strip over its active view (or the tabs
    /// shown side by side), or a blank area when it has none. Groups after the first draw
    /// the hairline divider on their leading edge. A tab dragged over a group drops at its end.
    pub(crate) fn render_group(&self, g: usize, cx: &mut Context<Self>) -> AnyElement {
        let [x0, y0, x1, y1] = self.views.layout.cell(g);
        let shown = self.views.shown(g);
        let dim = g != self.views.active;
        let views = div().flex_1().min_h_0().flex().flex_row().children(shown.into_iter().enumerate().map(|(k, e)| {
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .border_color(theme::tab_strip_bg(theme::GROUP_BORDER))
                .when(k > 0, |d| d.border_l(px(theme::GROUP_BORDER_W)))
                .child(e)
        }));
        let area = match self.views.groups[g].tabs.is_empty() {
            false => views,
            true => div()
                .flex_1()
                .bg(match dim {
                    true => ui(theme::INACTIVE_EDITOR_BG),
                    false => c(theme::EDITOR_BG),
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |ws, _, window, cx| {
                        ws.views.focus(g);
                        ws.focus_editor(window, cx);
                        cx.notify();
                    }),
                ),
        };
        div()
            .id(("group", g))
            .absolute()
            .left(relative(x0))
            .top(relative(y0))
            .w(relative(x1 - x0))
            .h(relative(y1 - y0))
            .flex()
            .flex_col()
            .border_color(theme::tab_strip_bg(theme::GROUP_BORDER))
            .when(x0 > 0., |d| d.border_l(px(theme::GROUP_BORDER_W)))
            .when(y0 > 0., |d| d.border_t(px(theme::GROUP_BORDER_W)))
            .when(self.show_tabs, |d| d.child(self.render_tab_bar(g, cx)))
            .child(
                area.id(("group-area", g))
                    .drag_over::<DraggedTabs>(|s, _, _, _| s.bg(ui(theme::DROP_TARGET_BG)))
                    .on_drop(cx.listener(move |ws, d: &DraggedTabs, window, cx| {
                        let end = ws.views.groups.get(g).map_or(0, |gr| gr.tabs.len());
                        ws.drop_tabs(d, g, end, window, cx);
                    })),
            )
            .into_any_element()
    }
}
