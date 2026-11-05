use super::*;

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::app::perf::frame_begin();
        let title = self.window_title(cx);
        if title != self.shown_title {
            window.set_window_title(&title);
            self.shown_title = title;
        }
        self.watch_full_screen(window, cx);
        // a newly selected tab scrolls into view in its strip
        self.tab_scroll.resize_with(self.views.groups.len(), Default::default);
        for (group, (scroll, shown)) in self.views.groups.iter().zip(&mut self.tab_scroll) {
            let active = (!group.tabs.is_empty()).then_some(group.active);
            if *shown != active {
                *shown = active;
                if let Some(i) = active {
                    scroll.scroll_to_item(i);
                }
            }
        }
        self.sync_dimming(cx);
        let width = f32::from(window.viewport_size().width);
        // a narrower window (or leaving full screen) never hides the editor or the drag strip
        let sidebar_full = self.sidebar.drawn_width(width);
        // sliding: the side bar is drawn at its full width, clipped to the share shown,
        // so its rows do not reflow; hit testing follows the clipped width
        let shown = self.sidebar_shown_fraction();
        self.sidebar_anim = (self.sidebar_anim.take()).filter(|(_, t0)| t0.elapsed() < view::SIDEBAR_SLIDE);
        if self.sidebar_anim.is_some() {
            window.request_animation_frame();
        }
        let sidebar_w = sidebar_full * shown;
        let groups: Vec<AnyElement> = (0..self.views.groups.len()).map(|g| self.render_group(g, cx)).collect();
        let tab_bar_h = if self.show_tabs { theme::TAB_BAR_H } else { 0. };
        let right_dock = self.render_dock(session::Dock::Right, window, cx);
        let right_room = width - sidebar_w - docks::EDITOR_MIN_W - self.git_sidebar_width(window);
        let outline_room = right_room
            - right_dock.as_ref().map_or(0., |_| {
                self.panel_place().map_or(0., |p| docks::drawn(p.width, right_room, docks::PANEL_MIN_W))
            });
        let outline = self.render_outline(outline_room.clamp(0., 240.), cx);
        let sidebar_shown = self.sidebar.visible && self.sidebar_anim.is_none();
        let sidebar_drawn = shown > 0.;
        let git_sidebar = self.render_git_sidebar(window, cx);
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(c(theme::EDITOR_BG))
            .font_family(theme::UI_FONT)
            .text_size(px(theme::UI_FONT_SIZE))
            .text_color(ui(theme::UI_FG))
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            // a dock's edge: measured from the window's own edges
            .on_drag_move(cx.listener(Self::drag_panel))
            .on_drag_move(cx.listener(Self::drag_git_sidebar))
            // the insertion marker goes unless a strip under the pointer puts it back
            .on_drag_move(cx.listener(|ws, _: &DragMoveEvent<DraggedTabs>, _, _| {
                ws.tab_drop = None;
            }))
            // let go outside the window: onto another Orlo window, or into a new one
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|ws, e: &gpui::MouseUpEvent, window, cx| {
                    let drag = cx.try_global::<TabDrag>().and_then(|d| d.0.clone());
                    if let Some(d) = drag.filter(|d| d.from == ws.window) {
                        ws.drop_outside(d, e.position, window, cx);
                    }
                }),
            )
            // let go over no drop target: nothing moves
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|ws, _: &gpui::MouseUpEvent, _, cx| {
                    if cx.try_global::<TabDrag>().is_some_and(|d| d.0.is_some()) || ws.tab_drop.is_some() {
                        ws.end_tab_drag(cx);
                    }
                }),
            )
            .when(crate::app::perf::enabled(), |d| d.capture_key_down(|_, _, _| crate::app::perf::input()))
            .map(|d| customization::bind_workspace_actions(d, cx))
            .child(
                div()
                    .id("body")
                    .relative()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .when(sidebar_drawn, |d| {
                        d.child(
                            div()
                                .id("sidebar-clip")
                                .w(px(sidebar_w))
                                .h_full()
                                .flex_shrink_0()
                                .overflow_hidden()
                                .child(sidebar::render(self, sidebar_full, cx)),
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .id("groups")
                                    .relative()
                                    .flex_1()
                                    .min_h_0()
                                    .child({
                                        // the area's size, for splits to check
                                        let cell = self.groups_bounds.clone();
                                        gpui::canvas(move |b, _, _| cell.set(b), |_, _, _, _| {}).absolute().size_full()
                                    })
                                    .children(groups)
                                    .children(self.render_dividers())
                                    .on_drag_move(cx.listener(Self::drag_divider)),
                            )
                            .when_some(self.quick_panel.as_ref(), |d, p| {
                                // just under the tab strips
                                d.child(
                                    div()
                                        .absolute()
                                        .top(px(tab_bar_h))
                                        .left_0()
                                        .right_0()
                                        .child(p.render(window, tab_bar_h, cx)),
                                )
                            }),
                    )
                    .children(outline)
                    .children(right_dock)
                    .children(git_sidebar)
                    .when(sidebar_shown, |d| {
                        d.child(
                            div()
                                .id("sidebar-edge")
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(px(sidebar_w - theme::DIVIDER_HIT / 2.))
                                .w(px(theme::DIVIDER_HIT))
                                .occlude()
                                .cursor(CursorStyle::ResizeLeftRight)
                                .on_drag(SidebarDrag, |_, _, _, cx| cx.new(|_| EmptyView)),
                        )
                    })
                    .on_drag_move(cx.listener(Self::drag_sidebar)),
            )
            // full width, under the side bar too, as in Sublime
            .children(self.render_dock(session::Dock::Bottom, window, cx))
            .children(self.render_context_menu(cx))
            .when(self.show_status_bar, |d| d.child(self.render_status_bar(cx)))
    }
}
