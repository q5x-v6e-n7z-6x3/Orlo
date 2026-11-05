//! Dynamic groups: splitting the active group, closing a group into its neighbours,
//! moving a view into a new group, saved layouts and synchronized scrolling.

use super::*;
use crate::chrome::layout::{load_saved, save_saved, saved_path, SavedLayout};

/// The smallest group a split may leave, in points: room for a tab strip and a few lines.
pub const SPLIT_MIN_W: f32 = 120.;
pub const SPLIT_MIN_H: f32 = 90.;

/// The saved layouts' names for View > Layout, read once and kept as layouts are saved and
/// deleted: the menus are rebuilt often and need not read the file each time.
pub(crate) struct SavedLayoutNames(pub(crate) Vec<String>);

impl gpui::Global for SavedLayoutNames {}

impl SavedLayoutNames {
    pub(crate) fn read() -> Self {
        Self(load_saved(&saved_path()).map_or_else(|_| Vec::new(), |(l, _)| names(&l)))
    }
}

fn names(list: &[SavedLayout]) -> Vec<String> {
    list.iter().map(|l| l.name.clone()).collect()
}

impl Workspace {
    /// The groups area as last drawn, `None` before the first frame.
    fn groups_area(&self) -> Option<(f32, f32)> {
        let b = self.groups_bounds.get();
        let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
        (w > 0. && h > 0.).then_some((w, h))
    }

    /// Split group `g` along `axis`; the new, empty group takes focus. Refused, with the layout
    /// untouched, when a half would be too small to use.
    fn split(&mut self, g: usize, axis: Axis, cx: &mut Context<Self>) -> bool {
        let Some(l) = self.views.layout.split(g, axis) else {
            return false;
        };
        if let Some((w, h)) = self.groups_area() {
            if !l.fits(w, h, SPLIT_MIN_W, SPLIT_MIN_H) {
                self.status_message("Not enough room to split this group", cx);
                return false;
            }
        }
        if !l.is_valid() {
            return false;
        }
        self.views.split(g, l);
        true
    }

    /// View > Groups > Split Right / Split Down: the active group in two.
    pub(crate) fn split_group(&mut self, a: &SplitGroup, window: &mut Window, cx: &mut Context<Self>) {
        if self.split(self.views.active, a.axis, cx) {
            self.groups_changed(window, cx);
        }
    }

    /// View > Groups > Move File to New Group Right / Down: the active view goes into a new
    /// group split off its own.
    pub(crate) fn move_to_new_group(&mut self, a: &MoveToNewGroup, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.views.active_tab().cloned() else {
            return;
        };
        if !self.split(self.views.active, a.axis, cx) {
            return;
        }
        // moved, not closed and added: a pinned view stays pinned
        self.views.move_tab(&view, self.views.active, 0);
        self.groups_changed(window, cx);
    }

    /// View > Groups > Close Group: its views go to the neighbour that grows over its place,
    /// unsaved ones included, its active view staying active there. Refused when no
    /// neighbour's edge covers it exactly (a pinwheel), or it is the only group.
    pub(crate) fn close_group(&mut self, _: &CloseGroup, window: &mut Window, cx: &mut Context<Self>) {
        let g = self.views.active;
        if self.views.groups.len() < 2 {
            return self.status_message("The only group stays", cx);
        }
        let Some((l, target)) = self.views.layout.close(g) else {
            let why = "No neighbour can take this group's place; choose a layout instead";
            return self.status_message(why, cx);
        };
        self.views.close(g, l, target);
        self.tab_scroll.clear();
        self.groups_changed(window, cx);
    }

    /// View > Sync Scrolling > Vertical / Horizontal (window-wide, kept in the session).
    pub(crate) fn toggle_sync_scroll(&mut self, a: &ToggleSyncScroll, _: &mut Window, cx: &mut Context<Self>) {
        let (on, which) = match a.vertical {
            true => (&mut self.sync_scroll.0, "Vertical"),
            false => (&mut self.sync_scroll.1, "Horizontal"),
        };
        *on ^= true;
        let on = *on;
        self.status_message(format!("{which} scroll sync {}", if on { "on" } else { "off" }), cx);
        if on {
            if let Some(e) = self.active_editor() {
                self.sync_scroll_from(&e, cx);
            }
        }
        self.refresh_menus(cx);
        autosave(cx);
    }

    /// The views on screen, including tabs shown side by side.
    pub(crate) fn visible_views(&self) -> Vec<Entity<Editor>> {
        (0..self.views.groups.len()).flat_map(|g| self.views.shown(g)).collect()
    }

    /// The user scrolled `source`: with sync on, the other views on screen go to its top line
    /// (clamped to their own length; wrapped or not, a line is a line) and its sideways
    /// scroll. They do not tell anyone in turn, so nothing echoes back.
    pub(crate) fn sync_scroll_from(&mut self, source: &Entity<Editor>, cx: &mut Context<Self>) {
        let (v, h) = self.sync_scroll;
        if !(v || h) {
            return;
        }
        let shown = self.visible_views();
        if !shown.contains(source) {
            return;
        }
        let (line, into, left) = source.read(cx).scroll_position(cx);
        for t in shown.iter().filter(|t| *t != source) {
            t.update(cx, |e, cx| {
                e.follow_scroll(v.then_some((line, into)), h.then_some(left), cx);
                cx.notify();
            });
        }
    }

    /// The saved layouts, or an alert saying why the file cannot be read.
    fn saved_layouts(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Vec<SavedLayout>> {
        match load_saved(&saved_path()) {
            Ok((list, bad)) => {
                cx.set_global(SavedLayoutNames(names(&list)));
                if !bad.is_empty() {
                    let detail = format!("These were left out:\n{}", bad.join("\n"));
                    self.alert(window, cx, "Some saved layouts cannot be used", &detail);
                }
                Some(list)
            }
            Err(e) => {
                self.alert(window, cx, "Could not read the saved layouts", &e);
                None
            }
        }
    }

    /// View > Layout > Save Layout…: the current groups' geometry under a name, asking before
    /// replacing one of that name.
    pub(crate) fn save_layout(&mut self, _: &SaveLayout, window: &mut Window, cx: &mut Context<Self>) {
        let layout = self.views.layout.clone();
        let done: OnDone = Box::new(move |v, _, ws, window, cx| {
            let name = v.text("text").trim().to_string();
            if name.is_empty() {
                return Err("A layout needs a name".into());
            }
            let Some(mut list) = ws.saved_layouts(window, cx) else {
                return Ok(());
            };
            list.retain(|l| l.name != name);
            list.push(SavedLayout { name: name.clone(), layout: layout.clone() });
            save_saved(&saved_path(), &list).map_err(|e| e.to_string())?;
            cx.set_global(SavedLayoutNames(names(&list)));
            ws.status_message(format!("Saved the layout \"{name}\""), cx);
            ws.refresh_menus(cx);
            Ok(())
        });
        let field = Field::text("text", "Layout name", "");
        self.show_window_form("Save the layout as", vec![field], done, window, cx);
    }

    /// View > Layout > a saved layout: its geometry, the views kept (fewer groups move theirs
    /// to the last one, as the presets do). One that is not valid is refused.
    pub(crate) fn use_saved_layout(&mut self, a: &UseSavedLayout, window: &mut Window, cx: &mut Context<Self>) {
        let Some(list) = self.saved_layouts(window, cx) else {
            return;
        };
        let Some(l) = list.into_iter().find(|l| l.name == a.name) else {
            return self.status_message(format!("No saved layout \"{}\"", a.name), cx);
        };
        if a.delete {
            return self.delete_saved_layout(&l.name, window, cx);
        }
        self.views.set_layout(l.layout);
        self.groups_changed(window, cx);
    }

    fn delete_saved_layout(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut list) = self.saved_layouts(window, cx) else {
            return;
        };
        list.retain(|l| l.name != name);
        match save_saved(&saved_path(), &list) {
            Ok(()) => {
                cx.set_global(SavedLayoutNames(names(&list)));
                self.status_message(format!("Deleted the layout \"{name}\""), cx)
            }
            Err(e) => self.alert(window, cx, "Could not save the layouts", &e.to_string()),
        }
        self.refresh_menus(cx);
    }

    /// View > Layout > Apply / Delete Saved Layout…: the list to pick from.
    pub(crate) fn choose_saved_layout(&mut self, a: &ChooseSavedLayout, window: &mut Window, cx: &mut Context<Self>) {
        let Some(list) = self.saved_layouts(window, cx) else {
            return;
        };
        if list.is_empty() {
            let detail = "Save one with View > Layout > Save Layout…";
            return self.inform(window, cx, "No saved layouts", detail);
        }
        let items = (list.into_iter())
            .map(|l| {
                quick_panel::Item::command(
                    l.name.clone(),
                    format!("{} groups", l.layout.groups()),
                    UseSavedLayout { name: l.name, delete: a.delete },
                )
            })
            .collect();
        self.open_quick_panel(Kind::Commands, items, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::window;
    use super::*;
    use gpui::{size, TestAppContext};

    fn open_three(ws: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
        ws.update_in(cx, |ws, window, cx| {
            for t in ["one", "two", "three"] {
                ws.new_file(&NewFile, window, cx);
                ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert(t)));
            }
        });
    }

    #[gpui::test]
    fn split_close_and_move_to_a_new_group_keep_every_unsaved_view(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        cx.simulate_resize(size(px(1000.), px(700.)));
        open_three(&ws, cx);
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            ws.move_to_new_group(&MoveToNewGroup { axis: Axis::Cols }, window, cx);
            assert_eq!(ws.views.groups.len(), 2);
            assert_eq!(ws.views.groups[1].tabs.len(), 1, "three moved right");
            assert_eq!(ws.views.active, 1);
            // nested: the right group split down, one tall group beside two stacked ones
            ws.split_group(&SplitGroup { axis: Axis::Rows }, window, cx);
            assert_eq!(ws.views.groups.len(), 3);
            assert!(ws.views.layout.is_valid());
            assert_eq!(ws.views.layout.cells[0], [0, 0, 1, 2]);
            // the tall group closes: the two stacked grow over it, its views go to the first
            ws.focus_group(&FocusGroup { group: 0 }, window, cx);
            ws.close_group(&CloseGroup, window, cx);
            assert_eq!(ws.views.groups.len(), 2);
            assert_eq!(ws.views.layout, Layout::preset(Preset::Rows(2)));
            let texts: Vec<String> = (ws.views.tabs()).map(|t| t.read(cx).doc(cx).text()).collect();
            assert_eq!(texts.len(), 3, "{texts:?}");
            assert!(ws.views.tabs().all(|t| t.read(cx).doc(cx).dirty));
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).text(), "two");
        });
    }

    #[gpui::test]
    fn the_layout_menu_reads_saved_layouts_once_and_follows_deletes(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let saved = |names: &[&str]| {
            let list: Vec<_> =
                (names.iter()).map(|n| SavedLayout { name: n.to_string(), layout: Layout::default() }).collect();
            save_saved(&saved_path(), &list).unwrap();
        };
        let listed = |ws: &Workspace, cx: &App| -> Vec<String> {
            let menus = ws.menus(cx);
            let view = menus.iter().find(|m| m.name == "View").unwrap();
            let layouts = (view.items.iter())
                .find_map(|i| match i {
                    MenuItem::Submenu(m) if m.name == "Layout" => Some(&m.items),
                    _ => None,
                })
                .unwrap();
            (layouts.iter())
                .filter_map(|i| match i {
                    MenuItem::Action { name, action, .. } if action.name() == UseSavedLayout::name_for_type() => {
                        Some(name.to_string())
                    }
                    _ => None,
                })
                .collect()
        };
        saved(&["Wide", "Tall"]);
        ws.update_in(cx, |ws, _, cx| {
            // as at launch, before anything was read
            if cx.has_global::<SavedLayoutNames>() {
                cx.remove_global::<SavedLayoutNames>();
            }
            ws.set_menus_now(cx);
            assert_eq!(listed(ws, cx), ["Wide", "Tall"]);
            // the file is not read again for each rebuild
            saved(&["Other"]);
            assert_eq!(listed(ws, cx), ["Wide", "Tall"]);
        });
        ws.update_in(cx, |ws, window, cx| {
            let delete = UseSavedLayout { name: "Other".into(), delete: true };
            ws.use_saved_layout(&delete, window, cx);
            assert!(listed(ws, cx).is_empty(), "read again, then deleted");
        });
    }

    #[gpui::test]
    fn a_pinned_view_moved_to_a_new_group_stays_pinned(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        cx.simulate_resize(size(px(1000.), px(700.)));
        open_three(&ws, cx);
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            let three = ws.active_editor().unwrap();
            ws.views.set_pinned(&three, true);
            ws.move_to_new_group(&MoveToNewGroup { axis: Axis::Cols }, window, cx);
            assert_eq!(ws.views.groups[1].tabs, std::slice::from_ref(&three));
            assert_eq!(ws.views.groups[1].pinned, std::slice::from_ref(&three));
            assert_eq!(ws.active_editor(), Some(three));
        });
    }

    #[gpui::test]
    fn a_split_too_small_to_use_is_refused_and_the_layout_kept(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        // Leave 300 points for the editor beside the folder sidebar.
        cx.simulate_resize(size(px(300. + theme::SIDEBAR_W), px(400.)));
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            ws.split_group(&SplitGroup { axis: Axis::Cols }, window, cx);
            assert_eq!(ws.views.groups.len(), 2, "two halves of about 150");
            ws.split_group(&SplitGroup { axis: Axis::Cols }, window, cx);
            assert_eq!(ws.views.groups.len(), 2, "75 is too narrow: refused");
            let before = ws.views.layout.clone();
            ws.close_group(&CloseGroup, window, cx);
            ws.close_group(&CloseGroup, window, cx);
            assert_eq!(ws.views.groups.len(), 1, "the only group stays");
            assert_ne!(ws.views.layout, before);
        });
    }

    #[gpui::test]
    fn synchronized_scrolling_follows_lines_clamps_and_never_echoes(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        cx.simulate_resize(size(px(1000.), px(500.)));
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert(&"long\n".repeat(400))));
            ws.move_to_new_group(&MoveToNewGroup { axis: Axis::Cols }, window, cx);
            ws.focus_group(&FocusGroup { group: 0 }, window, cx);
            ws.new_file(&NewFile, window, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert(&"short\n".repeat(50))));
            ws.toggle_sync_scroll(&ToggleSyncScroll { vertical: true }, window, cx);
        });
        cx.run_until_parked();
        let (short, long) =
            ws.read_with(cx, |ws, _| (ws.views.groups[0].tabs[0].clone(), ws.views.groups[1].tabs[0].clone()));
        ws.update_in(cx, |ws, _, cx| {
            long.update(cx, |e, cx| e.follow_scroll(Some((30, 0.)), None, cx));
            ws.sync_scroll_from(&long, cx);
            assert_eq!(short.read(cx).scroll_position(cx).0, 30);
            // past the short one's end: clamped to its last line
            long.update(cx, |e, cx| e.follow_scroll(Some((300, 0.)), None, cx));
            ws.sync_scroll_from(&long, cx);
            assert_eq!(short.read(cx).scroll_position(cx).0, 50);
            // horizontal is its own toggle
            long.update(cx, |e, cx| e.follow_scroll(None, Some(40.), cx));
            ws.sync_scroll_from(&long, cx);
            assert_eq!(short.read(cx).scroll_position(cx).2, 0.);
        });
        // a follower's paint emits nothing back: the leader stays where it was put
        cx.run_until_parked();
        ws.read_with(cx, |_, cx| assert_eq!(long.read(cx).scroll_position(cx).0, 300));
    }
    #[gpui::test]
    fn scroll_sync_includes_side_by_side_tabs(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert(&"line\n".repeat(200))));
            let other = ws.active_editor().unwrap();
            ws.new_file(&NewFile, window, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert(&"line\n".repeat(200))));
            let source = ws.active_editor().unwrap();
            ws.views.toggle_selected(0, 0);
            assert_eq!(ws.views.shown(0).len(), 2);
            ws.toggle_sync_scroll(&ToggleSyncScroll { vertical: true }, window, cx);
            other.update(cx, |e, cx| e.follow_scroll(Some((0, 0.)), None, cx));
            source.update(cx, |e, cx| e.follow_scroll(Some((30, 0.)), None, cx));
            ws.sync_scroll_from(&source, cx);
            assert_eq!(other.read(cx).scroll_position(cx).0, 30);
        });
    }
}
