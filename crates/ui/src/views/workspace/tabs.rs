//! Tabs beyond the strip: several tabs shown side by side, preview and pinned tabs,
//! dragging tabs within and between groups and windows, and the tab list.

use super::*;
use gpui::{AnyWindowHandle, SharedString, WeakEntity};

/// A tab drag: the views it carries in strip order (the grabbed tab, or every tab shown side
/// by side when it is one of them), the grabbed one, and the window it started in. Views are
/// named by entity, never by index: a tab closed mid-drag is simply not moved.
#[derive(Clone)]
pub struct DraggedTabs {
    pub views: Vec<WeakEntity<Editor>>,
    pub grabbed: WeakEntity<Editor>,
    pub from: AnyWindowHandle,
    pub title: SharedString,
}

/// What follows the pointer during a tab drag.
pub struct TabGhost(pub SharedString);

impl Render for TabGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(12.))
            .h(px(theme::TAB_H - 4.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .border_1()
            .border_color(theme::on_sheet(theme::TAB_DIVIDER))
            .bg(c(theme::EDITOR_BG))
            .shadow_lg()
            .text_color(theme::on_sheet(theme::TAB_ACTIVE_FG))
            .text_size(px(12.))
            .font_family(theme::UI_FONT)
            .opacity(0.92)
            .child(self.0.clone())
    }
}

/// The tab drag under way, app-wide: a release outside its window still finds it.
#[derive(Default)]
pub struct TabDrag(pub Option<DraggedTabs>);

impl gpui::Global for TabDrag {}

/// Which tabs a tab list shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TabScope {
    Group(usize),
    Window,
    AllWindows,
}

impl Workspace {
    /// The tab a tab command acts on: the one whose context menu is open, else the active one.
    pub(crate) fn tab_target(&mut self) -> Option<Entity<Editor>> {
        let menu = self.context_tab.take().and_then(|w| w.upgrade());
        menu.filter(|t| self.views.find(t).is_some()).or_else(|| self.active_editor())
    }

    /// An edited preview is kept: editing in any view of its document promotes it.
    pub(crate) fn promote_if_edited(&mut self, document: u64, cx: &mut Context<Self>) {
        let previews: Vec<Entity<Editor>> = (self.views.groups.iter())
            .filter_map(|g| g.preview.clone())
            .filter(|p| {
                let doc = p.read(cx).doc(cx);
                doc.id() == document && doc.dirty
            })
            .collect();
        for p in previews {
            self.views.promote(&p);
            cx.notify();
        }
    }

    /// Close `view` as its tab's `×` does (the side bar's OPEN FILES).
    pub fn close_view(&mut self, view: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        if self.views.find(&view).is_some() {
            self.request_close(view, After::Close, window, cx);
        }
    }

    /// Show `view` as a click on its tab does.
    pub fn show_view(&mut self, view: &Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((g, i)) = self.views.find(view) {
            self.select_view(g, i, window, cx);
        }
    }

    /// A tab's context menu > Close: that tab, with the unsaved prompt.
    pub(crate) fn close_tab_target(&mut self, _: &CloseTabTarget, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.tab_target() {
            self.request_close(t, After::Close, window, cx);
        }
    }

    /// Tab > Keep Open (`cmd-k enter`, a double click on the tab): a preview becomes a tab.
    pub(crate) fn keep_open(&mut self, _: &KeepOpen, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.tab_target() {
            self.views.promote(&t);
            autosave(cx);
            cx.notify();
        }
    }

    /// Tab > Pin Tab / Unpin Tab.
    pub(crate) fn toggle_pin(&mut self, _: &TogglePin, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.tab_target() {
            let pin = !self.views.is_pinned(&t);
            self.views.set_pinned(&t, pin);
            autosave(cx);
            self.refresh_menus(cx);
            cx.notify();
        }
    }

    /// Close `views` in turn: clean ones at once, each unsaved one with its prompt (Cancel stops
    /// the rest).
    pub(crate) fn close_views(&mut self, views: Vec<Entity<Editor>>, window: &mut Window, cx: &mut Context<Self>) {
        self.close_queue = views.iter().map(|v| v.downgrade()).collect();
        self.close_next_queued(window, cx);
    }

    /// The next view of the close queue: `request_close` comes back here when it is done.
    pub(crate) fn close_next_queued(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while !self.close_queue.is_empty() {
            let next = self.close_queue.remove(0);
            let Some(v) = next.upgrade().filter(|v| self.views.find(v).is_some()) else {
                continue;
            };
            if v.read(cx).doc(cx).unsaved() {
                return self.request_close(v, After::CloseQueue, window, cx);
            }
            self.close_editor(&v, window, cx);
        }
    }

    /// Tab > Close Other Tabs: the group's other tabs, pinned ones excepted.
    pub(crate) fn close_other_tabs(&mut self, _: &CloseOtherTabs, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tabs_where(|i, target| i != target, window, cx);
    }

    /// Tab > Close Tabs to the Right, pinned ones excepted.
    pub(crate) fn close_tabs_to_right(&mut self, _: &CloseTabsToRight, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tabs_where(|i, target| i > target, window, cx);
    }

    /// Close the unpinned tabs of the target tab's group whose index `pick` takes, given the
    /// target's.
    fn close_tabs_where(&mut self, pick: impl Fn(usize, usize) -> bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((g, target)) = self.tab_target().and_then(|t| self.views.find(&t)) else {
            return;
        };
        let tabs: Vec<_> = (self.views.groups[g].tabs.iter().enumerate())
            .filter(|(i, x)| pick(*i, target) && !self.views.is_pinned(x))
            .map(|(_, x)| x.clone())
            .collect();
        self.close_views(tabs, window, cx);
    }

    /// Tab > Add Next / Previous Tab to Selection: the nearest tab that way not yet shown joins
    /// the side-by-side view and takes the input.
    pub(crate) fn add_tab_to_selection(&mut self, a: &AddTabToSelection, window: &mut Window, cx: &mut Context<Self>) {
        let g = self.views.active;
        let group = &self.views.groups[g];
        let shown = self.views.shown(g);
        let n = group.tabs.len();
        let from = group.active;
        let next = (1..n)
            .map(|k| match a.forward {
                true => (from + k) % n,
                false => (from + n - k) % n,
            })
            .find(|&i| !shown.contains(&group.tabs[i]));
        if let Some(i) = next {
            self.views.toggle_selected(g, i);
            self.tabs_changed(window, cx);
        }
    }

    /// Tab > Focus Next / Previous Selected Tab: the input moves among the tabs shown.
    pub(crate) fn focus_selected_tab(&mut self, a: &FocusSelectedTab, window: &mut Window, cx: &mut Context<Self>) {
        let g = self.views.active;
        let shown = self.views.shown(g);
        let Some(active) = self.views.active_tab().cloned() else {
            return;
        };
        let Some(at) = shown.iter().position(|t| *t == active) else {
            return;
        };
        let n = shown.len();
        let next = &shown[match a.forward {
            true => (at + 1) % n,
            false => (at + n - 1) % n,
        }];
        if let Some(i) = self.views.groups[g].tabs.iter().position(|t| t == next) {
            self.select_view(g, i, window, cx);
        }
    }

    /// Tab > Deselect Other Tabs: back to the one tab with the input.
    pub(crate) fn deselect_tabs(&mut self, _: &DeselectTabs, window: &mut Window, cx: &mut Context<Self>) {
        let g = self.views.active;
        self.views.groups[g].selected.clear();
        self.tabs_changed(window, cx);
    }

    /// After the tabs shown changed: focus, outlines, redraw.
    pub(crate) fn tabs_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_quick_panel(cx);
        self.focus_editor(window, cx);
        self.refresh_matches(cx);
        self.refresh_menus(cx);
        cx.notify();
    }

    /// A click on tab `i` of group `g`: `⌘` adds or removes it from the side-by-side view,
    /// `⇧` shows the range, a double click keeps a preview, a plain click selects it.
    pub(crate) fn click_tab(
        &mut self,
        view: &Entity<Editor>,
        e: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((g, i)) = self.views.find(view) else {
            return;
        };
        let m = e.modifiers();
        if e.click_count() > 1 {
            self.views.promote(view);
            autosave(cx);
            return cx.notify();
        }
        if m.platform {
            self.views.toggle_selected(g, i);
            return self.tabs_changed(window, cx);
        }
        if m.shift {
            self.views.select_range(g, i);
            return self.tabs_changed(window, cx);
        }
        self.select_view(g, i, window, cx);
    }

    /// A side bar click: the file in the active group as a preview, which the next
    /// preview replaces unless it was edited, kept or pinned. A file open there already is
    /// just selected; one open elsewhere gets a new view.
    pub fn open_preview(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = crate::editing::text::resolve(&path);
        self.previewing.retain(|p, kind| *kind != PendingOpen::Cancelled || self.loading.contains(p));
        for kind in self.previewing.values_mut() {
            if *kind == PendingOpen::Preview {
                *kind = PendingOpen::Cancelled;
            }
        }
        let g = self.views.active;
        let here =
            (self.views.groups[g].tabs.iter()).any(|t| t.read(cx).doc(cx).path.as_deref() == Some(path.as_path()));
        if here || path.is_dir() {
            return self.open_path_here(path, window, cx);
        }
        let kind = self.previewing.entry(path.clone()).or_insert(PendingOpen::Preview);
        if *kind == PendingOpen::Cancelled {
            *kind = PendingOpen::Preview;
        }
        self.open_path_here(path.clone(), window, cx);
        let Some(new) = self.views.active_tab().cloned() else {
            return;
        };
        if new.read(cx).doc(cx).path.as_deref() != Some(path.as_path()) {
            return;
        }
        // Opening another group's existing buffer does not go through `opened`.
        if self.previewing.remove(&path) == Some(PendingOpen::Preview) {
            self.finish_preview(new, window, cx);
        }
    }

    pub(crate) fn pending_sidebar_open(&self, path: &Path) -> bool {
        matches!(self.previewing.get(path), Some(PendingOpen::Preview | PendingOpen::Kept))
    }

    pub(crate) fn revive_pending_open(&mut self, path: &Path) {
        if let Some(kind @ PendingOpen::Cancelled) = self.previewing.get_mut(path) {
            *kind = PendingOpen::Kept;
        }
    }

    /// A sidebar double click or Enter keeps the file even while its preview is loading.
    pub fn open_kept_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = crate::editing::text::resolve(&path);
        self.previewing.insert(path.clone(), PendingOpen::Kept);
        self.open_path_here(path.clone(), window, cx);
        if let Some(t) = self.active_editor().filter(|t| t.read(cx).doc(cx).path.as_ref() == Some(&path)) {
            self.previewing.remove(&path);
            self.views.promote(&t);
        }
    }

    pub(crate) fn finish_preview(&mut self, new: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((g, _)) = self.views.find(&new) else {
            return;
        };
        let old = self.views.groups[g]
            .preview
            .clone()
            .filter(|p| *p != new && !p.read(cx).read_state(cx).unsaved() && !self.views.is_pinned(p));
        let at = old.as_ref().and_then(|o| self.views.find(o));
        if let (Some(old), Some((_, at))) = (old, at) {
            self.close_editor(&old, window, cx);
            self.views.move_tab(&new, g, at);
        }
        self.views.groups[g].preview = Some(new);
        self.focus_editor(window, cx);
        cx.notify();
    }

    /// The views a drag of `view` carries: every tab shown side by side when it is one of them.
    fn drag_views(&self, view: &Entity<Editor>) -> Vec<Entity<Editor>> {
        let Some((g, _)) = self.views.find(view) else {
            return Vec::new();
        };
        let shown = self.views.shown(g);
        match shown.len() > 1 && shown.contains(view) {
            true => shown,
            false => vec![view.clone()],
        }
    }

    /// The payload of a drag starting on `view`'s tab.
    pub(crate) fn dragged(&self, view: &Entity<Editor>, cx: &App) -> DraggedTabs {
        let views = self.drag_views(view);
        let title = match views.len() {
            1 => view.read(cx).title(cx),
            n => format!("{} tabs", n),
        };
        DraggedTabs {
            views: views.iter().map(|v| v.downgrade()).collect(),
            grabbed: view.downgrade(),
            from: self.window,
            title: title.into(),
        }
    }

    /// Drop `d` into group `g` before tab `index`, in this window. With `⌥` held the tabs are
    /// copied as new views of the same buffers (a clone shares the text, it never copies it);
    /// else they move, keeping their buffer, undo, selections, scroll and unsaved text. Views
    /// closed meanwhile, and a drop onto where they already are, change nothing.
    pub(crate) fn drop_tabs(
        &mut self,
        d: &DraggedTabs,
        g: usize,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_tab_drag(cx);
        if g >= self.views.groups.len() {
            return;
        }
        let clone = window.modifiers().alt;
        let views: Vec<Entity<Editor>> =
            d.views.iter().filter_map(|w| w.upgrade()).filter(|v| self.views.find(v).is_some()).collect();
        if views.is_empty() {
            return;
        }
        if d.from != self.window {
            // a drag of another window's tabs reaches here only through `drop_outside`
            return;
        }
        let mut at = index;
        let mut placed = Vec::new();
        let mut grabbed = d.grabbed.upgrade();
        for v in &views {
            let v = match clone {
                true => {
                    let copy = another_view(v, &mut grabbed, cx);
                    self.wire(&copy, window, cx);
                    self.views.insert_unpinned_at(g, at, copy.clone());
                    copy
                }
                false => {
                    // `move_tab` counts `at` as the strip is before the move
                    if self.views.move_tab(v, g, at).is_none() {
                        continue;
                    }
                    v.clone()
                }
            };
            if let Some((_, i)) = self.views.find(&v) {
                at = i + 1;
            }
            self.views.promote(&v);
            placed.push(v);
        }
        self.show_placed(g, &placed, grabbed);
        self.drop_blank_untitled_in(g, cx);
        autosave(cx);
        self.tabs_changed(window, cx);
    }

    /// The tabs just dropped into group `g` are shown side by side when there are several, the
    /// grabbed one with the input.
    fn show_placed(&mut self, g: usize, placed: &[Entity<Editor>], grabbed: Option<Entity<Editor>>) {
        let input = grabbed.filter(|v| placed.contains(v)).or_else(|| placed.first().cloned());
        let Some(input) = input else { return };
        let group = &mut self.views.groups[g];
        group.selected = placed.iter().filter(|v| **v != input).cloned().collect();
        if let Some(i) = group.tabs.iter().position(|t| *t == input) {
            group.active = i;
        }
        self.views.active = g;
    }

    /// A lone blank untitled tab in group `g` gives way to tabs dropped there.
    fn drop_blank_untitled_in(&mut self, g: usize, cx: &mut Context<Self>) {
        let group = &self.views.groups[g];
        if group.tabs.len() > 1 {
            let blank: Vec<_> = (group.tabs.iter())
                .filter(|t| !self.views.is_pinned(t) && Self::is_blank_untitled(t, cx))
                .cloned()
                .collect();
            for b in blank {
                self.remove_view(&b, cx);
            }
        }
    }

    /// Take `views` out of this window for another one: out of their groups, their listeners
    /// here dropped, nothing closed or saved. The ones that were here, in order.
    pub(crate) fn release_views(
        &mut self,
        views: &[Entity<Editor>],
        cx: &mut Context<Self>,
    ) -> Vec<(Entity<Editor>, bool)> {
        let mut out = Vec::new();
        for v in views {
            if self.views.find(v).is_some() {
                let pinned = self.views.is_pinned(v);
                self.remove_view(v, cx);
                out.push((v.clone(), pinned));
            }
        }
        self.cancel_quick_panel(cx);
        autosave(cx);
        cx.notify();
        out
    }

    /// Views from another window into group `g` before tab `index`: wired to this window,
    /// pinned ones pinned again, shown side by side when there are several.
    pub(crate) fn adopt_views(
        &mut self,
        views: Vec<(Entity<Editor>, bool)>,
        g: usize,
        index: usize,
        grabbed: Option<Entity<Editor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let g = g.min(self.views.groups.len() - 1);
        let mut at = index;
        let mut placed = Vec::new();
        for (v, pinned) in views {
            self.wire(&v, window, cx);
            v.update(cx, |e, cx| {
                // this window's display settings; the view keeps its own choices
                e.hold_top(cx);
                e.view = self.view_for_editor(e, cx);
                cx.notify();
            });
            let (_, i) = self.views.insert_unpinned_at(g, at, v.clone());
            if pinned {
                self.views.set_pinned(&v, true);
            }
            at = self.views.find(&v).map_or(i + 1, |(_, i)| i + 1);
            placed.push(v);
        }
        self.show_placed(g, &placed, grabbed);
        self.drop_blank_untitled_in(g, cx);
        autosave(cx);
        self.tabs_changed(window, cx);
        window.activate_window();
    }

    /// A tab drag let go outside this window: onto another Orlo window, into the group
    /// under the pointer; elsewhere, into a new window there. The tabs leave this window only
    /// once they have somewhere to go; a window left with nothing closes after them.
    pub(crate) fn drop_outside(
        &mut self,
        d: DraggedTabs,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_tab_drag(cx);
        let views: Vec<Entity<Editor>> = d.views.iter().filter_map(|w| w.upgrade()).collect();
        if views.is_empty() || d.from != self.window {
            return;
        }
        let b = window.bounds();
        let titlebar = b.size.height - window.viewport_size().height;
        let screen = gpui::point(b.origin.x + at.x, b.origin.y + titlebar + at.y);
        let target = self
            .other_windows(cx)
            .find(|w| w.update(cx, |_, window, _| window.bounds().contains(&screen)).unwrap_or(false));
        let clone = window.modifiers().alt;
        let mut grabbed = d.grabbed.upgrade();
        let released = if clone {
            (views.iter())
                .filter(|v| self.views.find(v).is_some())
                .map(|v| (another_view(v, &mut grabbed, cx), self.views.is_pinned(v)))
                .collect()
        } else {
            self.release_views(&views, cx)
        };
        if released.is_empty() {
            return;
        }
        let landed = match target {
            Some(w) => w
                .update(cx, |ws, window, cx| {
                    let (g, i) = ws.drop_place(screen, window);
                    ws.adopt_views(released.clone(), g, i, grabbed.clone(), window, cx);
                })
                .is_ok(),
            None => {
                let bounds = [
                    f32::from(screen.x) - 60.,
                    f32::from(screen.y) - 20.,
                    f32::from(b.size.width),
                    f32::from(b.size.height - titlebar),
                ];
                adopt_in_new_window(&released, &grabbed, bounds, cx)
            }
        };
        if !clone {
            self.after_release(landed, released, grabbed, window, cx);
        }
    }

    /// Views released for another window: back where they were when it could not take them
    /// after all, else this window closes if that left it empty.
    fn after_release(
        &mut self,
        landed: bool,
        released: Vec<(Entity<Editor>, bool)>,
        grabbed: Option<Entity<Editor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !landed {
            let g = self.views.active;
            return self.adopt_views(released, g, usize::MAX, grabbed, window, cx);
        }
        self.close_if_emptied(window, cx);
    }

    /// The group under a screen point in this window and the index at its end; the active
    /// group's end when the point is not over one.
    fn drop_place(&self, screen: gpui::Point<gpui::Pixels>, window: &Window) -> (usize, usize) {
        let b = window.bounds();
        let titlebar = b.size.height - window.viewport_size().height;
        let local = gpui::point(screen.x - b.origin.x, screen.y - b.origin.y - titlebar);
        let area = self.groups_bounds.get();
        let g = (area.contains(&local))
            .then(|| {
                let fx = f32::from(local.x - area.origin.x) / f32::from(area.size.width);
                let fy = f32::from(local.y - area.origin.y) / f32::from(area.size.height);
                (0..self.views.groups.len()).find(|&g| {
                    let [x0, y0, x1, y1] = self.views.layout.cell(g);
                    (x0..=x1).contains(&fx) && (y0..=y1).contains(&fy)
                })
            })
            .flatten()
            .unwrap_or(self.views.active);
        (g, self.views.groups[g].tabs.len())
    }

    /// Tab > Move to New Window: the tab (or the tabs shown side by side) in a window of its
    /// own, as a drag out of the window does.
    pub(crate) fn move_to_new_window(&mut self, _: &MoveToNewWindow, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tab_target() else { return };
        let d = self.dragged(&t, cx);
        let views: Vec<Entity<Editor>> = d.views.iter().filter_map(|w| w.upgrade()).collect();
        let b = window.bounds();
        let released = self.release_views(&views, cx);
        let bounds = [
            f32::from(b.origin.x) + 30.,
            f32::from(b.origin.y) + 30.,
            f32::from(b.size.width),
            f32::from(window.viewport_size().height),
        ];
        let grabbed = Some(t);
        let landed = adopt_in_new_window(&released, &grabbed, bounds, cx);
        self.after_release(landed, released, grabbed, window, cx);
    }

    /// A window whose last tab left for another one and that has no folders closes, as
    /// Sublime's does; Orlo carries on in the other window.
    fn close_if_emptied(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let empty = self.views.tabs().next().is_none();
        if empty && self.roots.is_empty() && self.project.is_none() {
            self.save_if_last(window, cx);
            window.remove_window();
            return;
        }
        if empty {
            self.focus_editor(window, cx);
        }
        cx.notify();
    }

    /// The drag is over: its marker and app-wide record go.
    pub(crate) fn end_tab_drag(&mut self, cx: &mut Context<Self>) {
        self.tab_drop = None;
        cx.default_global::<TabDrag>().0 = None;
        cx.notify();
    }

    /// The tab list: the views of a group, this window or every window, with their
    /// states, a file name used twice shown with its folder; Enter goes to one, `⌘⌫` closes it.
    pub(crate) fn show_tab_list(&mut self, a: &ShowTabList, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.tab_items(a.scope, cx);
        self.open_quick_panel(Kind::Tabs(a.scope), items, window, cx);
    }

    /// The tab list's rows, built from the views themselves.
    pub(crate) fn tab_items(&self, scope: TabScope, cx: &App) -> Vec<quick_panel::Item> {
        // (window number, this workspace or another, group, view)
        let mut rows: Vec<(usize, usize, Entity<Editor>, String)> = Vec::new();
        let mut add = |w: usize, ws: &Workspace, only: Option<usize>| {
            for (g, group) in ws.views.groups.iter().enumerate() {
                if only.is_some_and(|o| o != g) {
                    continue;
                }
                let shown = ws.views.shown(g);
                for (i, t) in group.tabs.iter().enumerate() {
                    let mut states = Vec::new();
                    if i == group.active && g == ws.views.active {
                        states.push("active");
                    } else if shown.contains(t) {
                        states.push("shown");
                    }
                    if t.read(cx).doc(cx).unsaved() {
                        states.push("unsaved");
                    }
                    if group.pinned.contains(t) {
                        states.push("pinned");
                    }
                    if group.preview.as_ref() == Some(t) {
                        states.push("preview");
                    }
                    rows.push((w, g, t.clone(), states.join(", ")));
                }
            }
        };
        match scope {
            TabScope::Group(g) => add(0, self, Some(g)),
            TabScope::Window => add(0, self, None),
            TabScope::AllWindows => {
                add(1, self, None);
                for (k, w) in self.other_windows(cx).enumerate() {
                    if let Ok(ws) = w.read(cx) {
                        add(k + 2, ws, None);
                    }
                }
            }
        }
        // a name two files share is told apart by their folders; views of one buffer are
        // numbered
        let names: Vec<(String, Option<PathBuf>)> =
            (rows.iter()).map(|(_, _, t, _)| (t.read(cx).title(cx), t.read(cx).doc(cx).path.clone())).collect();
        let several_groups = self.views.groups.len() > 1;
        let mut seen_buffers: Vec<(Entity<Editor>, usize)> = Vec::new();
        (rows.iter().zip(&names))
            .map(|((w, g, t, states), (name, path))| {
                let e = t.read(cx);
                let twice = (names.iter()).any(|(n, p)| n == name && p != path);
                let folder = (e.doc(cx).path.as_deref())
                    .and_then(Path::parent)
                    .and_then(Path::file_name)
                    .map(|f| f.to_string_lossy().into_owned());
                let mut label = match (twice, folder) {
                    (true, Some(f)) => format!("{name} — {f}"),
                    _ => name.clone(),
                };
                let same = seen_buffers.iter_mut().find(|(b, _)| b.read(cx).same_document(e));
                match same {
                    Some((_, n)) => {
                        *n += 1;
                        label = format!("{label} (view {})", *n);
                    }
                    None => seen_buffers.push((t.clone(), 1)),
                }
                let place = match (scope, *w) {
                    (TabScope::AllWindows, w) => format!("Window {w} · Group {}", g + 1),
                    (TabScope::Window, _) if several_groups => format!("Group {}", g + 1),
                    _ => String::new(),
                };
                let path = e.doc(cx).path.as_deref().map(tilde).unwrap_or_default();
                let detail =
                    [place, path, states.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                quick_panel::Item::new(label, detail, ItemAction::View(t.entity_id()))
            })
            .collect()
    }

    /// The view `id` in any window: its window brought to the front and the view selected.
    pub(crate) fn activate_view(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.view_by_id(id) {
            return self.show_view(&t, window, cx);
        }
        if let Some(w) = self.owner_of(id, cx) {
            cx.defer(move |cx| {
                w.update(cx, |ws, window, cx| {
                    window.activate_window();
                    ws.activate_view(id, window, cx);
                })
                .ok();
            });
        }
    }

    /// This window's view `id`.
    fn view_by_id(&self, id: EntityId) -> Option<Entity<Editor>> {
        self.views.tabs().find(|t| t.entity_id() == id).cloned()
    }

    /// The other window whose tabs include the view `id`.
    fn owner_of(&self, id: EntityId, cx: &App) -> Option<WindowHandle<Workspace>> {
        self.other_windows(cx).find(|w| w.read(cx).is_ok_and(|ws| ws.view_by_id(id).is_some()))
    }

    /// Orlo's other windows.
    fn other_windows(&self, cx: &App) -> impl Iterator<Item = WindowHandle<Workspace>> {
        let this = self.window;
        (cx.windows().into_iter()).filter(move |w| *w != this).filter_map(|w| w.downcast::<Workspace>())
    }

    /// `⌘⌫` in the tab list: the highlighted view closes (asking first if it is unsaved) and
    /// the list follows. Elsewhere the key does what it did.
    pub(crate) fn close_listed(&mut self, _: &quick_panel::CloseItem, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.quick_panel.as_ref() else {
            return cx.propagate();
        };
        let Kind::Tabs(scope) = p.kind else {
            return cx.propagate();
        };
        let id = (p.filtered.get(p.selected)).and_then(|&i| match &p.items[i].action {
            ItemAction::View(id) => Some(*id),
            _ => None,
        });
        let at = p.selected;
        let Some(id) = id else { return };
        let Some(view) = self.view_by_id(id) else {
            if let Some(owner) = self.owner_of(id, cx) {
                cx.defer(move |cx| {
                    owner
                        .update(cx, |ws, window, cx| {
                            if let Some(view) = ws.view_by_id(id) {
                                ws.close_view(view, window, cx);
                            }
                        })
                        .ok();
                });
            }
            return;
        };
        self.request_close(view, After::Close, window, cx);
        // the list again, without it (an unsaved one is asked about first, and stays listed)
        cx.defer_in(window, move |ws, window, cx| {
            ws.show_tab_list(&ShowTabList { scope }, window, cx);
            if let Some(p) = ws.quick_panel.as_mut() {
                p.selected = at.min(p.filtered.len().saturating_sub(1));
            }
        });
    }
}

/// A new view of `v`'s buffer, standing in for `v` as the grabbed one.
fn another_view(v: &Entity<Editor>, grabbed: &mut Option<Entity<Editor>>, cx: &mut App) -> Entity<Editor> {
    let copy = v.update(cx, |src, cx| cx.new(|cx| src.another_view(cx)));
    if grabbed.as_ref() == Some(v) {
        *grabbed = Some(copy.clone());
    }
    copy
}

/// A new window at `bounds`, without the side bar, takes `views`; false when it could not.
fn adopt_in_new_window(
    views: &[(Entity<Editor>, bool)],
    grabbed: &Option<Entity<Editor>>,
    bounds: [f32; 4],
    cx: &mut App,
) -> bool {
    let st = WindowState { bounds: Some(bounds), sidebar_visible: false, ..Default::default() };
    open_window(Some(st), cx)
        .update(cx, |ws, window, cx| ws.adopt_views(views.to_vec(), 0, 0, grabbed.clone(), window, cx))
        .is_ok()
}

/// A close may finish after an unsaved prompt in another window; keep its open tab lists current.
pub(crate) fn refresh_tab_lists(cx: &mut App) {
    for window in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
        cx.defer(move |cx| {
            window
                .update(cx, |ws, _, cx| {
                    let Some(p) = ws.quick_panel.as_ref() else {
                        return;
                    };
                    let Kind::Tabs(scope) = p.kind else { return };
                    let (query, selected) = (p.query.clone(), p.selected);
                    let items = ws.tab_items(scope, cx);
                    if let Some(p) = ws.quick_panel.as_mut() {
                        p.set_items(Kind::Tabs(scope), items);
                        p.refilter(&query);
                        p.selected = selected.min(p.filtered.len().saturating_sub(1));
                    }
                    cx.notify();
                })
                .ok();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{settle_until, temp, two_windows, window};
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    /// Files `names` in a folder of their own (real paths).
    fn files(dir: &str, names: &[&str]) -> Vec<PathBuf> {
        let d = crate::editing::text::resolve(&temp(dir));
        (names.iter())
            .map(|n| {
                let p = d.join(n);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, format!("{n}\n")).unwrap();
                p
            })
            .collect()
    }

    fn titles(ws: &Workspace, g: usize, cx: &App) -> Vec<String> {
        (ws.views.groups[g].tabs.iter()).map(|t| t.read(cx).title(cx)).collect()
    }

    fn type_text(e: &Entity<Editor>, t: &str, cx: &mut App) {
        e.update(cx, |e, cx| e.edit(cx, |s| s.insert(t)));
    }

    fn open_all(ws: &mut Workspace, files: &[PathBuf], window: &mut Window, cx: &mut Context<Workspace>) {
        for p in files {
            ws.open_path(p.clone(), window, cx);
        }
    }

    /// `⌥` held, as an Option-drag's drop sees it.
    fn hold_option(window: &mut Window) {
        window.set_modifiers(gpui::Modifiers { alt: true, ..Default::default() });
    }

    /// A file big enough to be read in the background.
    fn write_large(path: &Path) {
        std::fs::write(path, "line\n".repeat((LOAD_IN_BACKGROUND / 5 + 1) as usize)).unwrap();
    }

    fn settle_loading(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
        settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.loading.is_empty()));
    }

    fn tab_for<'a>(ws: &'a Workspace, path: &Path, cx: &App) -> Option<&'a Entity<Editor>> {
        (ws.views.tabs()).find(|e| e.read(cx).doc(cx).path.as_deref() == Some(path))
    }

    #[gpui::test]
    fn adopting_a_view_preserves_its_scoped_display_preferences(cx: &mut TestAppContext) {
        let path = files("orlo-tabs-adopt-preferences", &["a.rs"])[0].clone();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(
                r#"{"overrides":[{"language":"Rust","settings":{"font_size":24,"show_minimap":false}}]}"#,
                window,
                cx,
            );
            ws.open_path(path, window, cx);
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).view.font_size, 24.);
            let released = ws.release_views(std::slice::from_ref(&editor), cx);
            ws.adopt_views(released, 0, 0, Some(editor.clone()), window, cx);
            assert_eq!(editor.read(cx).view.font_size, 24.);
            assert!(!editor.read(cx).view.minimap);
        });
    }

    #[gpui::test]
    fn tabs_shown_side_by_side_keep_the_layout_and_move_the_input(cx: &mut TestAppContext) {
        let f = files("orlo-tabs-multi", &["a.txt", "b.txt", "c.txt"]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            open_all(ws, &f, window, cx);
            let layout = ws.views.layout.clone();
            ws.select_tab(&SelectTab { index: 0 }, window, cx);
            ws.add_tab_to_selection(&AddTabToSelection { forward: true }, window, cx);
            assert_eq!(ws.views.shown(0).len(), 2);
            assert_eq!(ws.active_editor().unwrap().read(cx).title(cx), "b.txt");
            assert_eq!(ws.views.layout, layout, "no layout change");
            ws.focus_selected_tab(&FocusSelectedTab { forward: true }, window, cx);
            assert_eq!(ws.active_editor().unwrap().read(cx).title(cx), "a.txt");
            assert_eq!(ws.views.shown(0).len(), 2, "still both shown");
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            // the one without the input is dimmed, the input view is not
            let shown = ws.views.shown(0);
            let input = ws.active_editor().unwrap();
            for v in &shown {
                assert_eq!(v.read(cx).dimmed, *v != input);
            }
            // closing the input view: the other one shown takes it
            ws.close_tab(&CloseTab, window, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).title(cx), "b.txt");
            assert_eq!(ws.views.shown(0).len(), 1);
            ws.add_tab_to_selection(&AddTabToSelection { forward: true }, window, cx);
            ws.deselect_tabs(&DeselectTabs, window, cx);
            assert_eq!(ws.views.shown(0).len(), 1);
            // in a session the side-by-side view is not saved; tabs and layout are
            let st = ws.capture(window, true, &mut Texts::new(), cx);
            assert_eq!(st.groups[0].views.len(), 2);
        });
    }

    #[gpui::test]
    fn a_preview_is_reused_until_edited_or_kept_and_never_replaced_while_unsaved(cx: &mut TestAppContext) {
        let f = files("orlo-tabs-preview", &["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.open_preview(f[0].clone(), window, cx);
            ws.open_preview(f[1].clone(), window, cx);
            assert_eq!(titles(ws, 0, cx), ["b.txt"], "a was replaced");
            assert!(ws.views.is_preview(&ws.active_editor().unwrap()));
            // an edit keeps it
            type_text(&ws.active_editor().unwrap(), "x", cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            assert!(!ws.views.is_preview(&ws.active_editor().unwrap()));
            ws.open_preview(f[2].clone(), window, cx);
            assert_eq!(titles(ws, 0, cx), ["b.txt", "c.txt"]);
            // Keep Open keeps it too
            ws.keep_open(&KeepOpen, window, cx);
            ws.open_preview(f[3].clone(), window, cx);
            assert_eq!(titles(ws, 0, cx), ["b.txt", "c.txt", "d.txt"]);
            // a preview that is unsaved (its file deleted) is never replaced
            let d = ws.active_editor().unwrap();
            d.update(cx, |e, cx| e.update_doc(cx, |d| d.deleted = true));
            ws.open_preview(f[4].clone(), window, cx);
            assert_eq!(titles(ws, 0, cx), ["b.txt", "c.txt", "d.txt", "e.txt"]);
            // the session keeps which is a preview
            let st = ws.capture(window, true, &mut Texts::new(), cx);
            let flags: Vec<bool> = st.groups[0].views.iter().map(|v| v.preview).collect();
            assert_eq!(flags, [false, false, false, true]);
        });
    }

    #[gpui::test]
    fn pinned_tabs_stay_first_survive_bulk_closes_and_ask_before_closing_unsaved(cx: &mut TestAppContext) {
        let f = files("orlo-tabs-pinned", &["a.txt", "b.txt", "c.txt", "d.txt"]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            open_all(ws, &f, window, cx);
            ws.select_tab(&SelectTab { index: 2 }, window, cx);
            ws.toggle_pin(&TogglePin, window, cx);
            assert_eq!(titles(ws, 0, cx), ["c.txt", "a.txt", "b.txt", "d.txt"]);
            ws.select_tab(&SelectTab { index: 1 }, window, cx);
            ws.close_tabs_to_right(&CloseTabsToRight, window, cx);
            assert_eq!(titles(ws, 0, cx), ["c.txt", "a.txt"]);
            ws.close_other_tabs(&CloseOtherTabs, window, cx);
            assert_eq!(titles(ws, 0, cx), ["c.txt", "a.txt"], "a stays, c is pinned");
            ws.close_all(&CloseAll, window, cx);
            assert_eq!(titles(ws, 0, cx), ["c.txt"], "Close All Files spares it");
            // the session keeps the pin
            let st = ws.capture(window, true, &mut Texts::new(), cx);
            assert!(st.groups[0].views[0].pinned);
            // closing it on its own asks first when it is unsaved
            type_text(&ws.active_editor().unwrap(), "x", cx);
            ws.close_tab(&CloseTab, window, cx);
        });
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        ws.read_with(cx, |ws, cx| assert_eq!(titles(ws, 0, cx), ["c.txt"]));
    }

    #[gpui::test]
    fn dragging_reorders_moves_between_groups_and_ignores_closed_tabs(cx: &mut TestAppContext) {
        let f = files("orlo-tabs-drag", &["a.txt", "b.txt", "c.txt"]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            open_all(ws, &f, window, cx);
            let a = ws.views.groups[0].tabs[0].clone();
            type_text(&a, "edited ", cx);
            let d = ws.dragged(&a, cx);
            ws.drop_tabs(&d, 0, 3, window, cx);
            assert_eq!(titles(ws, 0, cx), ["b.txt", "c.txt", "a.txt"]);
            // into an empty group: the same view, unsaved text and undo with it
            ws.views.set_layout(Layout::preset(Preset::Columns(2)));
            ws.drop_tabs(&d, 1, 0, window, cx);
            assert_eq!(ws.views.groups[1].tabs, vec![a.clone()]);
            assert_eq!(a.read(cx).doc(cx).text(), "edited a.txt\n");
            a.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
            assert_eq!(a.read(cx).doc(cx).text(), "a.txt\n");
            // two tabs shown side by side move together
            ws.views.select(0, 0);
            ws.views.toggle_selected(0, 1);
            let b = ws.views.groups[0].tabs[0].clone();
            let d2 = ws.dragged(&b, cx);
            assert_eq!(d2.views.len(), 2);
            ws.drop_tabs(&d2, 1, 1, window, cx);
            assert_eq!(titles(ws, 1, cx), ["a.txt", "b.txt", "c.txt"]);
            assert_eq!(ws.views.shown(1).len(), 2, "still side by side");
            assert!(ws.views.groups[0].tabs.is_empty());
            // a tab closed during the drag: nothing moves, nothing breaks
            ws.views.groups[1].selected.clear();
            let c = ws.views.groups[1].tabs[2].clone();
            let d3 = ws.dragged(&c, cx);
            ws.close_editor(&c, window, cx);
            ws.drop_tabs(&d3, 0, 0, window, cx);
            assert!(ws.views.groups[0].tabs.is_empty());
            assert_eq!(ws.tab_drop, None);
        });
    }

    #[gpui::test]
    fn option_drag_keeps_the_grabbed_tabs_copy_active(cx: &mut TestAppContext) {
        let f = files("orlo-tabs-option-grabbed", &["a.txt", "b.txt"]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            open_all(ws, &f, window, cx);
            ws.views.select(0, 0);
            ws.views.toggle_selected(0, 1);
            let grabbed = ws.active_editor().unwrap();
            let drag = ws.dragged(&grabbed, cx);
            ws.views.set_layout(Layout::preset(Preset::Columns(2)));
            hold_option(window);
            ws.drop_tabs(&drag, 1, 0, window, cx);
            let active = ws.active_editor().unwrap();
            assert_ne!(active, grabbed);
            assert!(active.read(cx).same_document(grabbed.read(cx)));
            assert_eq!(titles(ws, 0, cx), ["a.txt", "b.txt"]);
            assert_eq!(titles(ws, 1, cx), ["a.txt", "b.txt"]);
            assert_eq!(ws.views.shown(1).len(), 2);
        });
    }

    #[gpui::test]
    fn option_drag_ignores_tabs_closed_during_the_drag(cx: &mut TestAppContext) {
        let f = files("orlo-tabs-option-closed", &["a.txt", "b.txt"]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            open_all(ws, &f, window, cx);
            let closed = ws.active_editor().unwrap();
            let drag = ws.dragged(&closed, cx);
            ws.close_editor(&closed, window, cx);
            ws.views.set_layout(Layout::preset(Preset::Columns(2)));
            hold_option(window);
            ws.drop_tabs(&drag, 1, 0, window, cx);
            assert!(ws.views.groups[1].tabs.is_empty());
            assert_eq!(titles(ws, 0, cx), ["a.txt"]);
        });
    }

    #[gpui::test]
    fn a_tab_moved_to_another_window_keeps_its_buffer_and_its_sibling_follows(cx: &mut TestAppContext) {
        let f = files("orlo-tabs-windows", &["a.txt"]);
        let (ws1, ws2, mut vcx1, mut vcx2) = two_windows(cx);
        let (a, clone) = ws1.update_in(&mut vcx1, |ws, window, cx| {
            ws.open_path(f[0].clone(), window, cx);
            let a = ws.active_editor().unwrap();
            type_text(&a, "x", cx);
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            (a, ws.active_editor().unwrap())
        });
        let released = ws1.update(&mut vcx1, |ws, cx| ws.release_views(std::slice::from_ref(&a), cx));
        ws2.update_in(&mut vcx2, |ws, window, cx| {
            ws.adopt_views(released, 0, 0, Some(a.clone()), window, cx);
            assert_eq!(ws.views.groups[0].tabs, vec![a.clone()], "the blank tab gave way");
        });
        // typed in window 2, the view left in window 1 follows: one buffer
        ws2.update(&mut vcx2, |ws, cx| {
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| e.edit(cx, |s| s.insert("y")));
        });
        vcx2.run_until_parked();
        ws1.read_with(&vcx1, |_, cx| {
            assert!(clone.read(cx).same_document(a.read(cx)));
            assert_eq!(clone.read(cx).doc(cx).text(), a.read(cx).doc(cx).text());
            assert!(clone.read(cx).doc(cx).dirty);
        });
    }

    #[gpui::test]
    fn moving_a_windows_last_tab_out_closes_it_and_orlo_carries_on(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("draft")));
            ws.move_to_new_window(&MoveToNewWindow, window, cx);
        });
        cx.run_until_parked();
        let windows = cx.windows();
        assert_eq!(windows.len(), 1, "the emptied window closed, the new one stays");
        let new = windows[0].downcast::<Workspace>().unwrap();
        let text = new.update(cx, |ws, _, cx| ws.active_editor().unwrap().read(cx).doc(cx).text());
        assert_eq!(text.unwrap(), "draft");
    }

    #[gpui::test]
    fn the_tab_list_and_open_files_tell_duplicates_and_views_apart(cx: &mut TestAppContext) {
        let f = files("orlo-tabs-list", &["x/a.txt", "y/a.txt", "b.txt"]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            open_all(ws, &f, window, cx);
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            ws.select_tab(&SelectTab { index: 0 }, window, cx);
            ws.toggle_pin(&TogglePin, window, cx);
            let items = ws.tab_items(TabScope::Window, cx);
            let labels: Vec<_> = items.iter().map(|i| i.label.clone()).collect();
            assert_eq!(labels, ["a.txt — x", "a.txt — y", "b.txt", "b.txt (view 2)"]);
            assert!(items[0].detail.contains("pinned") && items[0].detail.contains("active"));
            // Open Files: the same rows, from the same views
            ws.sidebar.show_open_files = true;
            let rows = sidebar::open_rows(ws, cx);
            let open: Vec<_> = (rows.iter())
                .filter_map(|r| match r {
                    sidebar::OpenEntry::View(v) => Some(v.label.clone()),
                    _ => None,
                })
                .collect();
            assert_eq!(open, ["a.txt — x", "a.txt — y", "b.txt", "b.txt (2)"]);
            // the list goes to a view, and closes one
            ws.show_tab_list(&ShowTabList { scope: TabScope::Window }, window, cx);
            let p = ws.quick_panel.as_mut().unwrap();
            p.selected = 2;
            ws.confirm_quick_panel(window, cx);
            assert_eq!(ws.active_editor().unwrap().read(cx).title(cx), "b.txt");
            ws.show_tab_list(&ShowTabList { scope: TabScope::Window }, window, cx);
            ws.quick_panel.as_mut().unwrap().selected = 1;
            ws.close_listed(&quick_panel::CloseItem, window, cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.views.tabs().count(), 3, "y/a.txt closed");
            let items = ws.quick_panel.as_ref().map(|p| p.items.len());
            assert_eq!(items, Some(3), "the list follows");
            assert!(!ws.views.tabs().any(|t| t.read(cx).doc(cx).path.as_ref() == Some(&f[1])));
        });
    }
    #[gpui::test]
    fn option_tearoff_keeps_source_view(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("first")));
            ws.new_file(&NewFile, window, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("second")));
            let original = ws.active_editor().unwrap();
            let drag = ws.dragged(&original, cx);
            hold_option(window);
            ws.drop_outside(drag, gpui::point(px(-10000.), px(-10000.)), window, cx);
            assert!(ws.views.find(&original).is_some(), "Option-drop must preserve the source view");
        });
    }
    #[gpui::test]
    fn all_tabs_closes_remote_view(cx: &mut TestAppContext) {
        let (ws1, ws2, mut c1, c2) = two_windows(cx);
        let remote = ws2.read_with(&c2, |ws, _| ws.active_editor().unwrap());
        let id = remote.entity_id();
        ws1.update_in(&mut c1, |ws, window, cx| {
            ws.show_tab_list(&ShowTabList { scope: TabScope::AllWindows }, window, cx);
            let p = ws.quick_panel.as_mut().unwrap();
            p.selected =
                p.filtered.iter().position(|&i| matches!(p.items[i].action, ItemAction::View(v) if v == id)).unwrap();
            ws.close_listed(&quick_panel::CloseItem, window, cx);
        });
        c1.run_until_parked();
        c2.run_until_parked();
        ws2.read_with(&c2, |ws, _| {
            assert!(ws.views.find(&remote).is_none(), "All Open Tabs must close the selected remote view")
        });
    }

    #[gpui::test]
    fn option_drop_keeps_the_last_source_tab_and_clones_its_buffer(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let original = ws.update(cx, |ws, cx| {
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved draft")));
            ws.active_editor().unwrap()
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            original.update(cx, |e, cx| e.sel(cx).set_selection(1, 4));
            ws.views.set_pinned(&original, true);
            let drag = ws.dragged(&original, cx);
            hold_option(window);
            ws.drop_outside(drag, point(px(-10000.), px(-10000.)), window, cx);
            assert!(ws.views.find(&original).is_some());
            assert!(ws.views.is_pinned(&original));
        });
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 2);
        let clone = ws.read_with(cx, |ws, cx| {
            let copy = ws.open_editors(cx).into_iter().find(|v| *v != original).unwrap();
            assert!(copy.read(cx).same_document(original.read(cx)));
            assert_eq!(copy.read(cx).state.selections, vec![crate::editing::text::Selection::new(1, 4)]);
            assert_eq!(copy.read(cx).doc(cx).text(), "unsaved draft");
            copy
        });
        ws.update(cx, |_, cx| original.update(cx, |e, cx| e.edit(cx, |s| s.insert("edit"))));
        cx.run_until_parked();
        clone.read_with(cx, |e, cx| assert_eq!(e.doc(cx).text(), "ueditved draft"));
    }

    #[gpui::test]
    fn all_tabs_preserves_remote_unsaved_prompts_and_refreshes_its_filter(cx: &mut TestAppContext) {
        let (ws1, ws2, mut c1, mut c2) = two_windows(cx);
        let remote = ws2.update(&mut c2, |ws, cx| {
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| {
                e.label = Some("remote draft".into());
                e.edit(cx, |s| s.insert("keep this draft"));
            });
            e
        });
        let id = remote.entity_id();
        ws1.update_in(&mut c1, |ws, window, cx| {
            ws.show_tab_list(&ShowTabList { scope: TabScope::AllWindows }, window, cx);
            let p = ws.quick_panel.as_mut().unwrap();
            p.refilter("remote draft");
            assert_eq!(p.filtered.len(), 1);
            ws.close_listed(&quick_panel::CloseItem, window, cx);
        });
        c1.run_until_parked();
        assert!(c2.has_pending_prompt());
        c2.simulate_prompt_answer("Cancel");
        c2.run_until_parked();
        ws2.read_with(&c2, |ws, _| assert!(ws.views.find(&remote).is_some()));
        ws1.update_in(&mut c1, |ws, window, cx| ws.close_listed(&quick_panel::CloseItem, window, cx));
        c1.run_until_parked();
        c2.simulate_prompt_answer("Don't Save");
        c2.run_until_parked();
        c1.run_until_parked();
        ws2.read_with(&c2, |ws, _| assert!(ws.views.find(&remote).is_none()));
        ws1.read_with(&c1, |ws, _| {
            let p = ws.quick_panel.as_ref().unwrap();
            assert_eq!(p.query, "remote draft");
            assert!(p.filtered.is_empty());
            assert!(!p.items.iter().any(|i| matches!(i.action, ItemAction::View(v) if v == id)));
        });
    }
    #[gpui::test]
    fn a_background_preview_replaces_the_previous_preview(cx: &mut TestAppContext) {
        let f = files("orlo-large-preview", &["old.txt", "large.txt", "next.txt"]);
        write_large(&f[1]);
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_preview(f[0].clone(), window, cx);
            ws.open_preview(f[1].clone(), window, cx);
        });
        settle_loading(&ws, vcx);
        ws.update_in(vcx, |ws, window, cx| {
            let large = ws.active_editor().unwrap();
            assert_eq!(large.read(cx).doc(cx).path.as_ref(), Some(&f[1]));
            assert!(ws.views.is_preview(&large));
            assert_eq!(titles(ws, 0, cx), ["large.txt"]);
            ws.open_preview(f[2].clone(), window, cx);
            assert_eq!(titles(ws, 0, cx), ["next.txt"]);
        });
    }
    #[gpui::test]
    fn a_superseded_background_preview_does_not_replace_the_latest(cx: &mut TestAppContext) {
        let f = files("orlo-preview-order", &["old.txt", "large.txt", "latest.txt"]);
        write_large(&f[1]);
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_preview(f[0].clone(), window, cx);
            ws.open_preview(f[1].clone(), window, cx);
            ws.open_preview(f[2].clone(), window, cx);
            let captured = ws.capture(window, true, &mut Texts::new(), cx);
            assert!(
                captured.groups.iter().flat_map(|g| &g.views).all(|v| v.path != f[1]),
                "hot exit must not resurrect a superseded preview still being read"
            );
        });
        settle_loading(&ws, vcx);
        ws.read_with(vcx, |ws, cx| {
            assert_eq!(titles(ws, 0, cx), ["latest.txt"]);
            assert!(ws.views.is_preview(&ws.active_editor().unwrap()));
        });
    }

    #[gpui::test]
    fn a_pending_preview_can_be_kept(cx: &mut TestAppContext) {
        let f = files("orlo-preview-keep", &["old.txt", "large.txt", "latest.txt"]);
        write_large(&f[1]);
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_preview(f[0].clone(), window, cx);
            ws.open_preview(f[1].clone(), window, cx);
            // The sidebar's double-click path, before the background preview arrives.
            ws.open_kept_path(f[1].clone(), window, cx);
            ws.open_preview(f[2].clone(), window, cx);
        });
        settle_loading(&ws, vcx);
        ws.read_with(vcx, |ws, cx| {
            let large = tab_for(ws, &f[1], cx).unwrap();
            assert!(!ws.views.is_preview(large), "double-click keeps even a preview still being loaded");
            let latest = tab_for(ws, &f[2], cx).unwrap();
            assert!(ws.views.is_preview(latest), "browsing continues without replacing the kept file");
            assert_eq!(ws.views.tabs().count(), 2);
        });
    }

    #[gpui::test]
    fn a_superseded_preview_can_still_be_opened_explicitly(cx: &mut TestAppContext) {
        let f = files("orlo-preview-reopen", &["large.txt", "latest.txt"]);
        write_large(&f[0]);
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_preview(f[0].clone(), window, cx);
            ws.open_preview(f[1].clone(), window, cx);
            ws.open_path(f[0].clone(), window, cx);
        });
        settle_loading(&ws, vcx);
        ws.read_with(vcx, |ws, cx| {
            let large = tab_for(ws, &f[0], cx);
            assert!(large.is_some(), "an explicit Open revives the cancelled preview request");
            assert!(!ws.views.is_preview(large.unwrap()));
        });
    }
}
