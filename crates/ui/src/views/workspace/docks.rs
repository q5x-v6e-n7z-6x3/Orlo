//! Panels as docks: the results browser and the regex playground resize by their edge
//! and sit along the bottom or the right; any panel hides, collapses to its title and takes or
//! gives back the keys. A panel moved, hidden or collapsed is the same panel: its fields,
//! results, selection and the search or Apply under way carry on.

use super::*;
use crate::app::session::{Dock, PanelPlace};

/// The smallest a docked panel is drawn, and the room the editor always keeps.
pub const PANEL_MIN_H: f32 = 80.;
pub const PANEL_MIN_W: f32 = 220.;
pub const EDITOR_MIN_H: f32 = 120.;
pub const EDITOR_MIN_W: f32 = 200.;

/// gpui's drag payload for a dock's edge.
#[derive(Clone, Copy)]
pub struct PanelDrag(pub Dock);

/// `chosen` drawn in a window that leaves `room` for the panel: at least `min` unless the room
/// is smaller, so the editor stays usable.
pub fn drawn(chosen: f32, room: f32, min: f32) -> f32 {
    chosen.min(room).max(min.min(room)).max(0.)
}

impl Workspace {
    /// Which of `panel_places` the open panel has, when it is one that moves and resizes:
    /// results, playground, terminal, changes.
    fn panel_slot(&self) -> Option<usize> {
        match self.panel.as_ref()? {
            Panel::Results(_) => Some(0),
            Panel::Playground(_) => Some(1),
            Panel::Terminal(_) => Some(2),
            Panel::Changes(_) | Panel::Comparison(_) | Panel::History(_) | Panel::Rescue(_) => Some(3),
            _ => None,
        }
    }

    /// The open panel's place, when it is one that moves and resizes.
    pub(crate) fn panel_place(&self) -> Option<&PanelPlace> {
        let p = &self.panel_places;
        Some([&p.results, &p.playground, &p.terminal, &p.changes][self.panel_slot()?])
    }

    fn panel_place_mut(&mut self) -> Option<&mut PanelPlace> {
        let i = self.panel_slot()?;
        let p = &mut self.panel_places;
        [&mut p.results, &mut p.playground, &mut p.terminal, &mut p.changes].into_iter().nth(i)
    }

    /// The status bar's height, when it shows.
    fn status_bar_h(&self) -> f32 {
        match self.show_status_bar {
            true => theme::STATUS_BAR_H,
            false => 0.,
        }
    }

    /// The open panel's name, for its collapsed strip and messages.
    fn panel_name(&self) -> &'static str {
        match &self.panel {
            Some(Panel::Find(_)) => "Find",
            Some(Panel::FindInFiles(_)) => "Find in Files",
            Some(Panel::Form(_)) => "Form",
            Some(Panel::Results(_)) => "Search Results",
            Some(Panel::Playground(_)) => "Regex Playground",
            Some(Panel::Terminal(_)) => "Terminal",
            Some(Panel::Changes(_)) => "Change",
            Some(Panel::Comparison(_)) => "Comparison",
            Some(Panel::History(_)) => "File History",
            Some(Panel::Rescue(_)) => "Rescue",
            None => "",
        }
    }

    /// Where the keys go in the open panel: its list or its first field.
    fn panel_focus(&self, cx: &App) -> Option<FocusHandle> {
        match &self.panel {
            Some(Panel::Find(p)) => Some(p.query.read(cx).focus_handle(cx)),
            Some(Panel::FindInFiles(p)) => Some(p.fields()[0].read(cx).focus_handle(cx)),
            Some(Panel::Form(p)) => p.inputs().next().map(|e| e.read(cx).focus_handle(cx)),
            Some(Panel::Results(p)) => Some(p.focus.clone()),
            Some(Panel::Playground(p)) => Some(p.pattern.read(cx).focus_handle(cx)),
            Some(Panel::Terminal(t)) => Some(t.read(cx).focus_handle(cx)),
            Some(Panel::Changes(p)) => Some(p.viewer.read(cx).focus_handle(cx)),
            Some(Panel::Comparison(p)) => {
                Some(p.viewers().first().map_or_else(|| p.focus.clone(), |e| e.read(cx).focus_handle(cx)))
            }
            Some(Panel::History(p)) => Some(p.viewer.read(cx).focus_handle(cx)),
            Some(Panel::Rescue(p)) => Some(p.viewer.read(cx).focus_handle(cx)),
            None => None,
        }
    }

    /// Whether the keys are in the open panel.
    fn panel_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focused_input(window, cx).is_some()
            || matches!(&self.panel, Some(Panel::Results(p)) if p.focus.is_focused(window))
            || matches!(&self.panel, Some(Panel::Comparison(p)) if p.focus.contains_focused(window, cx))
            || matches!(&self.panel, Some(Panel::Terminal(t)) if t.read(cx).focus_handle(cx).contains_focused(window, cx))
    }

    /// Hidden or collapsed with the keys in it: they go back to the editor, never to a control
    /// no one can see.
    pub(crate) fn keys_out_of_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel_has_focus(window, cx) {
            self.focus_editor(window, cx);
        }
    }

    /// View > Panel > Show / Hide Panel (`cmd-k cmd-j`): the panel stays as it is while hidden.
    /// With none open, the last search's results browser shows.
    pub(crate) fn toggle_panel(&mut self, _: &TogglePanel, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.is_none() {
            if self.result_set.is_some() {
                return self.show_results(&ShowResults, window, cx);
            }
            return self.status_message("No panel to show", cx);
        }
        self.panel_hidden ^= true;
        match self.panel_hidden {
            true => self.keys_out_of_panel(window, cx),
            false => {
                self.panel_collapsed = false;
                if let Some(f) = self.panel_focus(cx) {
                    window.focus(&f);
                }
            }
        }
        self.refresh_menus(cx);
        cx.notify();
    }

    /// View > Panel > Collapse / Expand Panel: down to its title strip and back.
    pub(crate) fn toggle_collapse_panel(
        &mut self,
        _: &ToggleCollapsePanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.panel.is_none() {
            return self.status_message("No panel to collapse", cx);
        }
        self.panel_hidden = false;
        self.panel_collapsed ^= true;
        if self.panel_collapsed {
            self.keys_out_of_panel(window, cx);
        }
        self.refresh_menus(cx);
        cx.notify();
    }

    /// View > Panel > Move Panel to Bottom / Right: the results browser and the playground.
    pub(crate) fn move_panel(&mut self, a: &MovePanel, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.panel, Some(Panel::Terminal(_))) && a.dock == Dock::Right {
            return self.status_message("The terminal stays at the bottom", cx);
        }
        let name = self.panel_name();
        match self.panel_place_mut() {
            Some(p) => p.dock = a.dock,
            None if name.is_empty() => return self.status_message("No panel to move", cx),
            None => {
                let why = format!("{name} stays along the bottom; the results browser and the regex playground move");
                return self.status_message(why, cx);
            }
        }
        (self.panel_hidden, self.panel_collapsed) = (false, false);
        autosave(cx);
        self.refresh_menus(cx);
        cx.notify();
    }

    /// View > Panel > Focus Panel: the keys to the panel, or back to the editor from it.
    pub(crate) fn focus_panel(&mut self, _: &FocusPanel, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel_has_focus(window, cx) {
            self.focus_editor(window, cx);
            return cx.notify();
        }
        (self.panel_hidden, self.panel_collapsed) = (false, false);
        match self.panel_focus(cx) {
            Some(f) => window.focus(&f),
            None => self.status_message("No panel to focus", cx),
        }
        cx.notify();
    }

    /// A drag of a dock's edge chooses its size; drawing clamps it to the window.
    pub(crate) fn drag_panel(&mut self, e: &DragMoveEvent<PanelDrag>, _: &mut Window, cx: &mut Context<Self>) {
        let PanelDrag(dock) = *e.drag(cx);
        let (b, p) = (e.bounds, e.event.position);
        let status = self.status_bar_h();
        let Some(place) = self.panel_place_mut() else {
            return;
        };
        match dock {
            Dock::Bottom => place.height = (f32::from(b.bottom() - p.y) - status).max(PANEL_MIN_H),
            Dock::Right => place.width = f32::from(b.right() - p.x).max(PANEL_MIN_W),
        }
        cx.notify();
    }

    /// The panel along `dock`, if it is there and showing: an input bar at its own height, a
    /// tool panel at its size (clamped so the editor keeps room) with an edge to drag, or a
    /// collapsed title strip.
    pub(crate) fn render_dock(&mut self, dock: Dock, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.panel.is_none() || self.panel_hidden {
            return None;
        }
        let place = self.panel_place().copied();
        if place.map_or(Dock::Bottom, |p| p.dock) != dock {
            return None;
        }
        if self.panel_collapsed {
            let name = self.panel_name();
            return Some(
                div()
                    .id("panel-collapsed")
                    .h(px(26.))
                    .flex_shrink_0()
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .bg(ui(theme::PANEL_BG))
                    .border_t_1()
                    .border_color(ui(theme::BORDER))
                    .text_color(ui(theme::UI_FG))
                    .cursor_pointer()
                    .hover(|s| s.bg(ui(theme::ELEMENT_HOVER)))
                    .child(icon(Icon::ChevronRight, 12.).text_color(ui(theme::ICON_MUTED)))
                    .child(name)
                    .on_click(cx.listener(|ws, _: &ClickEvent, window, cx| {
                        ws.toggle_collapse_panel(&ToggleCollapsePanel, window, cx)
                    }))
                    .into_any_element(),
            );
        }
        let body = self.render_panel(cx)?;
        let Some(place) = place else {
            return Some(body);
        };
        let view = window.viewport_size();
        let edge =
            div().id("panel-edge").absolute().occlude().on_drag(PanelDrag(dock), |_, _, _, cx| cx.new(|_| EmptyView));
        Some(match dock {
            Dock::Bottom => {
                let room = f32::from(view.height) - self.status_bar_h() - EDITOR_MIN_H;
                let h = drawn(place.height, room, PANEL_MIN_H);
                div()
                    .relative()
                    .flex_shrink_0()
                    .h(px(h))
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(body)
                    .child(
                        edge.top(px(-theme::DIVIDER_HIT / 2.))
                            .left_0()
                            .right_0()
                            .h(px(theme::DIVIDER_HIT))
                            .cursor(CursorStyle::ResizeUpDown),
                    )
                    .into_any_element()
            }
            Dock::Right => {
                let sidebar = match self.sidebar.visible {
                    true => self.sidebar.drawn_width(f32::from(view.width)),
                    false => 0.,
                };
                let room = f32::from(view.width) - sidebar - EDITOR_MIN_W - self.git_sidebar_width(window);
                let w = drawn(place.width, room, PANEL_MIN_W);
                div()
                    .relative()
                    .flex_shrink_0()
                    .w(px(w))
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_l_1()
                    .border_color(ui(theme::BORDER))
                    .child(body)
                    .child(
                        edge.left(px(-theme::DIVIDER_HIT / 2.))
                            .top_0()
                            .bottom_0()
                            .w(px(theme::DIVIDER_HIT))
                            .cursor(CursorStyle::ResizeLeftRight),
                    )
                    .into_any_element()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::window;
    use super::*;
    use gpui::{size, TestAppContext};

    #[test]
    fn a_dock_keeps_the_editor_room_and_its_own_minimum() {
        assert_eq!(drawn(300., 1000., 80.), 300.);
        assert_eq!(drawn(900., 400., 80.), 400., "clamped to the room");
        assert_eq!(drawn(10., 400., 80.), 80., "never under its minimum");
        assert_eq!(drawn(300., 50., 80.), 50., "a tiny window: the room wins");
        assert_eq!(drawn(300., -20., 80.), 0.);
    }

    #[gpui::test]
    fn hiding_moving_and_collapsing_keep_the_panel_and_send_the_keys_back(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        cx.simulate_resize(size(px(1000.), px(700.)));
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("abc abc")));
            ws.open_playground(&OpenPlayground, window, cx);
            let pattern = match &ws.panel {
                Some(Panel::Playground(p)) => p.pattern.clone(),
                _ => panic!("the playground"),
            };
            pattern.update(cx, |e, cx| e.set_text("b", cx));
            pattern.read(cx).focus(window);
            // hidden: the keys go to the editor, the panel and its text stay
            ws.toggle_panel(&TogglePanel, window, cx);
            assert!(ws.panel_hidden && !pattern.read(cx).is_focused(window));
            assert!(ws.active_editor().unwrap().read(cx).is_focused(window));
            ws.toggle_panel(&TogglePanel, window, cx);
            assert!(!ws.panel_hidden && pattern.read(cx).is_focused(window));
            assert_eq!(pattern.read(cx).text(cx), "b");
            // to the right and back; collapsed and back
            ws.move_panel(&MovePanel { dock: Dock::Right }, window, cx);
            assert_eq!(ws.panel_places.playground.dock, Dock::Right);
            ws.toggle_collapse_panel(&ToggleCollapsePanel, window, cx);
            assert!(!pattern.read(cx).is_focused(window));
            ws.focus_panel(&FocusPanel, window, cx);
            assert!(!ws.panel_collapsed && pattern.read(cx).is_focused(window));
            assert_eq!(pattern.read(cx).text(cx), "b", "the same panel throughout");
            // the Find panel does not move
            ws.find(&Find, window, cx);
            ws.move_panel(&MovePanel { dock: Dock::Right }, window, cx);
            assert!(ws.panel_place().is_none());
        });
        cx.run_until_parked();
        // the places go with the window's session
        ws.update_in(cx, |ws, window, cx| {
            ws.panel_places.results.height = 9999.;
            let mut texts = Texts::new();
            let st = ws.capture(window, true, &mut texts, cx);
            assert_eq!(st.panels.playground.dock, Dock::Right);
            assert_eq!(st.panels.results.height, 9999., "the choice; drawing clamps it");
        });
    }
    #[gpui::test]
    fn find_reopens_hidden_panel(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.find(&Find, window, cx);
            ws.toggle_panel(&TogglePanel, window, cx);
            assert!(ws.panel_hidden);
            ws.find(&Find, window, cx);
            assert!(ws.focused_input(window, cx).is_some());
            assert!(!ws.panel_hidden, "Find focused an input while its panel remained hidden");
        });
    }

    #[gpui::test]
    fn reopening_each_panel_reveals_its_existing_controls(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            for panel in 0..4 {
                let open = |ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| match panel {
                    0 => ws.find(&Find, window, cx),
                    1 => ws.find_in_files(&FindInFiles, window, cx),
                    2 => ws.show_results_panel(window, cx),
                    _ => ws.open_playground(&OpenPlayground, window, cx),
                };
                open(ws, window, cx);
                for collapsed in [false, true] {
                    if collapsed {
                        ws.toggle_collapse_panel(&ToggleCollapsePanel, window, cx);
                    } else {
                        ws.toggle_panel(&TogglePanel, window, cx);
                    }
                    open(ws, window, cx);
                    assert!(!ws.panel_hidden && !ws.panel_collapsed, "panel {panel}");
                    assert!(ws.panel_has_focus(window, cx));
                }
            }
        });
    }

    #[gpui::test]
    fn panel_shortcut_reopens_results_from_the_editor(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            ws.show_results_panel(window, cx);
            ws.toggle_panel(&TogglePanel, window, cx);
            ws.focus_editor(window, cx);
            assert!(ws.panel_hidden);
        });
        vcx.simulate_keystrokes("cmd-k cmd-j");
        ws.read_with(vcx, |ws, _| {
            assert!(!ws.panel_hidden, "the results panel reopens from the editor");
            assert!(matches!(ws.panel, Some(Panel::Results(_))));
        });
        vcx.simulate_keystrokes("cmd-k cmd-j");
        ws.read_with(vcx, |ws, _| assert!(ws.panel_hidden));
    }

    #[gpui::test]
    fn right_dock_leaves_editor_room_with_only_open_files_sidebar(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        vcx.simulate_resize(size(px(800.), px(600.)));
        ws.update_in(vcx, |ws, window, cx| {
            ws.toggle_open_files(&ToggleOpenFiles, window, cx);
            ws.open_playground(&OpenPlayground, window, cx);
            ws.move_panel(&MovePanel { dock: Dock::Right }, window, cx);
            ws.panel_places.playground.width = 9999.;
            cx.notify();
        });
        vcx.run_until_parked();
        ws.read_with(vcx, |ws, _| {
            assert!(
                f32::from(ws.groups_bounds.get().size.width) >= EDITOR_MIN_W - 1.,
                "the Open Files sidebar also consumes window width"
            );
        });
    }
}
