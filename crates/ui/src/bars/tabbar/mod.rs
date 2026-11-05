//! The tab strip and tab switching.

use crate::workspace::*;

impl Workspace {
    /// The active group's tab count; tab commands do nothing in an empty group.
    fn active_tabs(&self) -> usize {
        self.views.groups[self.views.active].tabs.len()
    }

    /// The tab `to(selected, count)` names in the active group.
    fn step_tab(&mut self, to: fn(usize, usize) -> usize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.active_tabs();
        if n > 0 {
            let index = to(self.views.groups[self.views.active].active, n);
            self.select_tab(&SelectTab { index }, window, cx);
        }
    }

    forward_actions! {
    pub(crate) next_tab(NextTab, window, cx) => step_tab(|i, n| (i + 1) % n, window, cx);
    pub(crate) prev_tab(PrevTab, window, cx) => step_tab(|i, n| (i + n - 1) % n, window, cx);
    pub(crate) last_tab(LastTab, window, cx) => step_tab(|_, n| n - 1, window, cx);
    }

    pub(crate) fn select_tab(&mut self, a: &SelectTab, window: &mut Window, cx: &mut Context<Self>) {
        if a.index < self.active_tabs() {
            self.select_view(self.views.active, a.index, window, cx);
        }
    }

    /// Focus tab `i` of group `g` (a tab click in any group).
    pub(crate) fn select_view(&mut self, g: usize, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_quick_panel(cx);
        self.views.select(g, i);
        self.focus_editor(window, cx);
        self.refresh_matches(cx);
        cx.notify();
    }

    /// Group `g`'s tab strip: a step darker than the colour scheme, flat tabs filling it, the
    /// selected one in the group's editor background. Tabs shrink to their labels, then the
    /// strip scrolls, keeping the selected tab in view. Tabs shown side by side look selected;
    /// the input tab of the focused group (or of tabs side by side) carries an accent line
    /// along its top. A preview's name is in italics, a pinned tab shows a pin. Tabs drag
    /// within and between strips and groups, and out of the window; `▾` lists the group's tabs.
    pub(crate) fn render_tab_bar(&self, g: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let group = &self.views.groups[g];
        let sheet = match g == self.views.active {
            true => c(theme::EDITOR_BG),
            false => ui(theme::INACTIVE_EDITOR_BG),
        };
        let shown = self.views.shown(g);
        let several = shown.len() > 1;
        let focused = g == self.views.active;
        struct Tab {
            editor: Entity<Editor>,
            title: String,
            dirty: bool,
            deleted: bool,
            shown: bool,
            preview: bool,
            pinned: bool,
        }
        // Sublime draws no tab for a window's lone blank untitled view, until it has text
        let tabs: Vec<Tab> = match group.tabs.as_slice() {
            [only] if Self::is_blank_untitled(only, cx) => Vec::new(),
            all => (all.iter())
                .map(|t| {
                    let e = t.read(cx);
                    Tab {
                        editor: t.clone(),
                        title: format!("{}{}", e.title(cx), if e.doc(cx).viewer_mode() { " [VIEWER]" } else { "" }),
                        dirty: e.doc(cx).dirty,
                        deleted: e.doc(cx).deleted,
                        shown: shown.contains(t),
                        preview: group.preview.as_ref() == Some(t),
                        pinned: group.pinned.contains(t),
                    }
                })
                .collect(),
        };
        let n = tabs.len();
        let active_at = (!group.tabs.is_empty()).then_some(group.active);
        let drop_at = self.tab_drop.filter(|(dg, _)| *dg == g).map(|(_, i)| i);
        // the strip's own buttons: a quiet glyph that brightens, on a wash, under the mouse
        let strip_button = |id: &'static str, i: Icon, tip: &'static str| {
            div()
                .id(id)
                .group("strip-button")
                .flex_shrink_0()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(kit::RADIUS))
                .cursor_pointer()
                .hover(|s| s.bg(theme::on_sheet(theme::TAB_BUTTON_HOVER_BG)))
                .child(
                    icon(i, 14.)
                        .text_color(theme::on_sheet(theme::NEW_TAB))
                        .group_hover("strip-button", |s| s.text_color(theme::on_sheet(theme::TAB_GLYPH_HOVER))),
                )
                .tooltip(kit::tooltip(tip.into()))
        };
        let scroll = self.tab_scroll.get(g).map(|s| s.0.clone()).unwrap_or_default();
        let edge_scroll = scroll.clone();
        let marker = || div().absolute().top(px(4.)).w(px(2.)).h(px(theme::TAB_H - 6.)).bg(c(theme::DROP_MARK));
        div()
            .id(("tab-strip", g))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .h(px(theme::TAB_BAR_H))
            .w_full()
            .pr(px(6.))
            .bg(theme::tab_strip_bg(theme::TAB_BAR_BG))
            // a drag over the strip drops at its end unless a tab says where (capture order:
            // the strip before its tabs); near either end the strip scrolls
            .on_drag_move(cx.listener(move |ws, e: &DragMoveEvent<DraggedTabs>, _, cx| {
                let (b, p) = (e.bounds, e.event.position);
                if !b.contains(&p) {
                    return;
                }
                ws.tab_drop = Some((g, n));
                let edge = px(28.);
                let step = if p.x < b.left() + edge {
                    px(12.)
                } else if p.x > b.right() - edge {
                    px(-12.)
                } else {
                    px(0.)
                };
                if step != px(0.) {
                    let o = edge_scroll.offset();
                    edge_scroll.set_offset(point(o.x + step, o.y));
                }
                cx.notify();
            }))
            .on_drop(cx.listener(move |ws, d: &DraggedTabs, window, cx| {
                let at = ws.tab_drop.filter(|(dg, _)| *dg == g).map_or(n, |(_, i)| i);
                ws.drop_tabs(d, g, at, window, cx);
            }))
            .child(
                div()
                    .id(("tabs", g))
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_x_scroll()
                    .track_scroll(&scroll)
                    .children(tabs.into_iter().enumerate().map(|(i, tab)| {
                        // clicks name the tab drawn, not its index: after a close the
                        // neighbour slides under the pointer before the next click
                        let editor = tab.editor;
                        let close = editor.clone();
                        let menu = editor.clone();
                        let drag = self.dragged(&editor, cx);
                        let input = active_at == Some(i);
                        let lit = input || tab.shown;
                        // a hairline between two inactive neighbours
                        let divider = !lit && i + 1 < n && active_at != Some(i + 1);
                        let glyph = match (tab.deleted, lit) {
                            // a file gone from disk shows a red dot, dirty or not
                            (true, _) => c(theme::DELETED_DOT),
                            (false, true) => theme::on_sheet(theme::TAB_CLOSE),
                            (false, false) => theme::on_sheet(theme::TAB_CLOSE_INACTIVE),
                        };
                        let fg = theme::on_sheet(match lit {
                            true => theme::TAB_ACTIVE_FG,
                            false => theme::TAB_INACTIVE_FG,
                        });
                        div()
                            .id(("tab", i))
                            .group("tab")
                            .relative()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .flex_basis(px(if tab.pinned { 120. } else { 170. }))
                            .h(px(theme::TAB_H))
                            .pl(px(14.))
                            .pr(px(8.))
                            .when(lit, |d| d.bg(sheet))
                            .when(!lit, |d| {
                                d.hover(|s| {
                                    s.bg(theme::on_sheet(theme::TAB_HOVER_BG))
                                        .text_color(theme::on_sheet(theme::TAB_ACTIVE_FG))
                                })
                            })
                            .text_size(px(12.))
                            .text_color(fg)
                            .cursor_pointer()
                            .on_click(
                                cx.listener(move |ws, e: &ClickEvent, window, cx| ws.click_tab(&editor, e, window, cx)),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |ws, e: &gpui::MouseDownEvent, _, cx| {
                                    ws.open_tab_menu(&menu, e.position, cx)
                                }),
                            )
                            .on_drag(drag, |d: &DraggedTabs, _, _, cx| {
                                cx.default_global::<TabDrag>().0 = Some(d.clone());
                                let title = d.title.clone();
                                cx.new(|_| TabGhost(title))
                            })
                            // where in the strip it would go: before or after this tab
                            .on_drag_move(cx.listener(move |ws, e: &DragMoveEvent<DraggedTabs>, _, cx| {
                                let (b, p) = (e.bounds, e.event.position);
                                if b.contains(&p) {
                                    let after = p.x > b.left() + b.size.width / 2.;
                                    ws.tab_drop = Some((g, i + after as usize));
                                    cx.notify();
                                }
                            }))
                            .when(input && (several || focused), |d| {
                                d.child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(theme::sheet_accent()))
                            })
                            .when(drop_at == Some(i), |d| d.child(marker().left(px(-1.))))
                            .when(tab.pinned, |d| d.child(icon(Icon::Pin, 11.).text_color(fg)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .when(tab.preview, |d| d.italic())
                                    .child(tab.title),
                            )
                            .child(
                                // Zed's tabs: the close button shows on the tab in front and
                                // under the mouse; an unsaved file keeps its dot
                                div()
                                    .id(("tab-close", i))
                                    .group("tab-close")
                                    .debug_selector(move || format!("tab-close-{i}"))
                                    .flex_shrink_0()
                                    .size(px(18.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(kit::RADIUS))
                                    .hover(|s| s.bg(theme::on_sheet(theme::TAB_BUTTON_HOVER_BG)))
                                    .map(|d| match tab.dirty || tab.deleted {
                                        true => d.child(div().size(px(7.)).rounded_full().bg(glyph)),
                                        false => d
                                            .child(
                                                icon(Icon::Close, 11.)
                                                    .text_color(glyph)
                                                    .group_hover("tab-close", |s| {
                                                        s.text_color(theme::on_sheet(theme::TAB_GLYPH_HOVER))
                                                    }),
                                            )
                                            .when(!lit, |d| d.opacity(0.).group_hover("tab", |s| s.opacity(1.))),
                                    })
                                    .on_click(cx.listener(move |ws, e: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        let open = ws.views.find(&close).is_some();
                                        if e.click_count() == 1 && open {
                                            ws.request_close(close.clone(), After::Close, window, cx);
                                        }
                                    })),
                            )
                            .when(divider, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .right_0()
                                        .top(px((theme::TAB_H - 16.) / 2.))
                                        .w(px(1.))
                                        .h(px(16.))
                                        .bg(theme::on_sheet(theme::TAB_DIVIDER)),
                                )
                            })
                    }))
                    .when(drop_at == Some(n), |d| d.child(div().relative().w(px(2.)).child(marker()))),
            )
            .child(strip_button("tab-list", Icon::ChevronDown, "Show Tab List").on_click(cx.listener(
                move |ws, _: &ClickEvent, window, cx| {
                    ws.views.focus(g);
                    let a = ShowTabList { scope: TabScope::Group(g) };
                    ws.show_tab_list(&a, window, cx)
                },
            )))
            .child(strip_button("new-tab", Icon::Plus, "New File").on_click(cx.listener(
                move |ws, _: &ClickEvent, window, cx| {
                    ws.views.focus(g);
                    ws.new_file(&NewFile, window, cx)
                },
            )))
    }

    /// A tab's right-click menu: its commands act on that tab.
    fn open_tab_menu(&mut self, view: &Entity<Editor>, at: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        use crate::workspace::context_menu::item;
        self.context_tab = Some(view.downgrade());
        let pinned = self.views.is_pinned(view);
        let preview = self.views.is_preview(view);
        let mut items = vec![
            item("Close", CloseTabTarget),
            item("Close Other Tabs", CloseOtherTabs),
            item("Close Tabs to the Right", CloseTabsToRight),
            None,
            item(if pinned { "Unpin Tab" } else { "Pin Tab" }, TogglePin),
        ];
        if preview {
            items.push(item("Keep Open", KeepOpen));
        }
        items.extend([None, item("New View into File", NewViewIntoFile), item("Move to New Window", MoveToNewWindow)]);
        self.open_context_menu(at, items, cx);
    }
}
