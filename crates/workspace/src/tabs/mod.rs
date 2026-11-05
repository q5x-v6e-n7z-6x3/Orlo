//! Generic tab ordering, multi-selection, previews, and pinned tab management.

use crate::panes::Groups;

#[derive(Clone, Debug, PartialEq)]
pub struct Group<T> {
    pub tabs: Vec<T>,
    pub active: usize,
    /// Tabs shown side by side with the active one, never the active one itself.
    pub selected: Vec<T>,
    /// The transient tab the next preview replaces.
    pub preview: Option<T>,
    /// Pinned tabs, always the first tabs of the strip.
    pub pinned: Vec<T>,
}

impl<T> Default for Group<T> {
    fn default() -> Self {
        Self { tabs: Vec::new(), active: 0, selected: Vec::new(), preview: None, pinned: Vec::new() }
    }
}

impl<T: PartialEq + Clone> Groups<T> {
    pub fn active_tab(&self) -> Option<&T> {
        let g = &self.groups[self.active];
        g.tabs.get(g.active)
    }

    /// Appended to the active group and made active.
    pub fn add(&mut self, tab: T) {
        let g = &mut self.groups[self.active];
        g.tabs.push(tab);
        g.active = g.tabs.len() - 1;
    }

    /// At `i` in group `g` (clamped), the group's active tab staying the same tab.
    pub fn insert_at(&mut self, g: usize, i: usize, tab: T) -> (usize, usize) {
        let g = g.min(self.groups.len() - 1);
        let group = &mut self.groups[g];
        let i = i.min(group.tabs.len());
        if !group.tabs.is_empty() && i <= group.active {
            group.active += 1;
        }
        group.tabs.insert(i, tab);
        (g, i)
    }

    /// As `insert_at`, but never in front of the group's pinned tabs.
    pub fn insert_unpinned_at(&mut self, g: usize, i: usize, tab: T) -> (usize, usize) {
        let g = g.min(self.groups.len() - 1);
        let i = i.max(self.groups[g].pinned.len());
        self.insert_at(g, i, tab)
    }

    /// Right after the active tab of the active group, made active.
    pub fn insert_next(&mut self, tab: T) {
        let g = &mut self.groups[self.active];
        let at = if g.tabs.is_empty() { 0 } else { g.active + 1 };
        g.tabs.insert(at, tab);
        g.active = at;
    }

    pub fn find(&self, tab: &T) -> Option<(usize, usize)> {
        self.groups.iter().enumerate().find_map(|(gi, g)| g.tabs.iter().position(|t| t == tab).map(|i| (gi, i)))
    }

    /// Removing the active tab activates the one that slides into its place (the one before
    /// it, at the end), or another of the tabs shown side by side; the group may become empty.
    pub fn remove(&mut self, tab: &T) -> Option<(usize, usize)> {
        let (gi, i) = self.find(tab)?;
        let g = &mut self.groups[gi];
        let was_active = i == g.active;
        g.tabs.remove(i);
        g.selected.retain(|t| t != tab);
        g.pinned.retain(|t| t != tab);
        if g.preview.as_ref() == Some(tab) {
            g.preview = None;
        }
        if i < g.active || g.active >= g.tabs.len() {
            g.active = g.active.saturating_sub(1);
        }
        // the input moves to another of the tabs shown
        if was_active && !g.selected.is_empty() {
            let next = g.selected.remove(0);
            g.active = g.tabs.iter().position(|t| *t == next).unwrap_or(g.active);
        }
        Some((gi, i))
    }

    /// Focus group `g` and its tab `i` (clamped); no-op if the group does not exist. A tab shown
    /// side by side takes the input and the others stay shown; any other tab ends the
    /// side-by-side view.
    pub fn select(&mut self, g: usize, i: usize) {
        if let Some(group) = self.groups.get_mut(g) {
            let i = i.min(group.tabs.len().saturating_sub(1));
            if let (Some(new), Some(old)) = (group.tabs.get(i), group.tabs.get(group.active)) {
                if group.selected.contains(new) {
                    let (new, old) = (new.clone(), old.clone());
                    group.selected.retain(|t| *t != new);
                    group.selected.push(old);
                } else if *new != *old {
                    group.selected.clear();
                }
            }
            group.active = i;
            self.active = g;
        }
    }

    /// The active view goes to the end of group `g`, and focus follows it.
    pub fn move_active_to(&mut self, g: usize) {
        if g >= self.groups.len() || g == self.active {
            return;
        }
        let Some(tab) = self.active_tab().cloned() else {
            return;
        };
        let end = self.groups[g].tabs.len();
        self.move_tab(&tab, g, end);
    }

    /// Every tab, in group order.
    pub fn tabs(&self) -> impl Iterator<Item = &T> {
        self.groups.iter().flat_map(|g| g.tabs.iter())
    }

    /// `⌘`-click on tab `i` of group `g`: shown beside the others and given the input;
    /// on a tab already shown, it leaves the side-by-side view (the input moving to another).
    pub fn toggle_selected(&mut self, g: usize, i: usize) {
        let Some(group) = self.groups.get_mut(g) else {
            return;
        };
        let (Some(tab), Some(active)) = (group.tabs.get(i).cloned(), group.tabs.get(group.active).cloned()) else {
            return;
        };
        self.active = g;
        if tab == active {
            if !group.selected.is_empty() {
                let next = group.selected.remove(0);
                group.active = group.tabs.iter().position(|t| *t == next).unwrap_or(i);
            }
        } else if group.selected.contains(&tab) {
            group.selected.retain(|t| *t != tab);
        } else {
            group.selected.push(active);
            group.active = i;
        }
    }

    /// `⇧`-click on tab `i` of group `g`: every tab from the active one to it shown side by
    /// side, the clicked one with the input.
    pub fn select_range(&mut self, g: usize, i: usize) {
        let Some(group) = self.groups.get_mut(g) else {
            return;
        };
        if i >= group.tabs.len() {
            return;
        }
        let (a, b) = (group.active.min(i), group.active.max(i));
        group.selected = (a..=b).filter(|&k| k != i).map(|k| group.tabs[k].clone()).collect();
        group.active = i;
        self.active = g;
    }

    /// Group `g`'s tabs on screen, in strip order: the active one and those shown beside it.
    pub fn shown(&self, g: usize) -> Vec<T> {
        let Some(group) = self.groups.get(g) else {
            return Vec::new();
        };
        (group.tabs.iter().enumerate())
            .filter(|(k, t)| *k == group.active || group.selected.contains(t))
            .map(|(_, t)| t.clone())
            .collect()
    }

    /// Move `tab` to group `to` before the tab now at `index` there (clamped, and kept behind
    /// the pinned tabs unless it is pinned, among them if it is), made active; focus follows.
    /// It stops being a preview. `None` when the tab or group is gone.
    pub fn move_tab(&mut self, tab: &T, to: usize, mut index: usize) -> Option<(usize, usize)> {
        if to >= self.groups.len() {
            return None;
        }
        let (from, i) = self.find(tab)?;
        let pinned = self.groups[from].pinned.contains(tab);
        // within one group the indices after its old place move down
        if from == to && i < index {
            index -= 1;
        }
        self.remove(tab);
        let dest = &mut self.groups[to];
        dest.selected.clear();
        let (lo, hi) = match pinned {
            true => (0, dest.pinned.len()),
            false => (dest.pinned.len(), dest.tabs.len()),
        };
        let index = index.clamp(lo, hi);
        dest.tabs.insert(index, tab.clone());
        if pinned {
            dest.pinned.push(tab.clone());
            dest.sort_pinned();
        }
        dest.active = index;
        self.active = to;
        Some((to, index))
    }

    /// Pin or unpin `tab`: pinned tabs move to the end of the pinned ones, unpinned
    /// ones just after them. A pinned tab is no preview.
    pub fn set_pinned(&mut self, tab: &T, pin: bool) {
        let Some((g, i)) = self.find(tab) else { return };
        let group = &mut self.groups[g];
        if group.pinned.contains(tab) == pin {
            return;
        }
        let active = group.tabs.get(group.active).cloned();
        group.tabs.remove(i);
        match pin {
            true => {
                let at = group.pinned.len();
                group.tabs.insert(at, tab.clone());
                group.pinned.push(tab.clone());
                if group.preview.as_ref() == Some(tab) {
                    group.preview = None;
                }
            }
            false => {
                group.pinned.retain(|t| t != tab);
                group.tabs.insert(group.pinned.len(), tab.clone());
            }
        }
        group.sort_pinned();
        if let Some(a) = active {
            group.active = group.tabs.iter().position(|t| *t == a).unwrap_or(0);
        }
    }

    pub fn is_pinned(&self, tab: &T) -> bool {
        self.groups.iter().any(|g| g.pinned.contains(tab))
    }

    pub fn is_preview(&self, tab: &T) -> bool {
        self.groups.iter().any(|g| g.preview.as_ref() == Some(tab))
    }

    /// `tab` is kept open: no preview any more.
    pub fn promote(&mut self, tab: &T) {
        for g in &mut self.groups {
            if g.preview.as_ref() == Some(tab) {
                g.preview = None;
            }
        }
    }
}

impl<T: PartialEq + Clone> Group<T> {
    /// The pinned tabs first, in their order on the strip; the others after, in theirs.
    pub(crate) fn sort_pinned(&mut self) {
        let active = self.tabs.get(self.active).cloned();
        let (mut first, rest): (Vec<T>, Vec<T>) = self.tabs.drain(..).partition(|t| self.pinned.contains(t));
        self.pinned = first.clone();
        first.extend(rest);
        self.tabs = first;
        if let Some(a) = active {
            self.active = self.tabs.iter().position(|t| *t == a).unwrap_or(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unpinned_insert_goes_behind_the_pinned_tabs() {
        let mut g = Groups::new();
        for t in ["a", "b", "c"] {
            g.add(t);
        }
        g.set_pinned(&"b", true);
        g.set_pinned(&"c", true);
        g.select(0, 2);
        assert_eq!(g.insert_unpinned_at(0, 0, "copy"), (0, 2));
        assert_eq!(g.groups[0].tabs, vec!["b", "c", "copy", "a"]);
        assert_eq!(g.active_tab(), Some(&"a"));
        assert_eq!(g.insert_unpinned_at(0, 9, "end"), (0, 4));
        assert_eq!(g.groups[0].pinned, vec!["b", "c"]);
    }
}
