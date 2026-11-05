//! Pane groups, layout transitions, and focus across the editor area.

use crate::layout::{Layout, Preset};
use crate::tabs::Group;

#[derive(Clone, Debug, PartialEq)]
pub struct Groups<T> {
    pub groups: Vec<Group<T>>,
    pub active: usize,
    pub layout: Layout,
}

impl<T: PartialEq + Clone> Default for Groups<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: PartialEq + Clone> Groups<T> {
    pub fn new() -> Self {
        Self { groups: vec![Group::default()], active: 0, layout: Layout::preset(Preset::Single) }
    }

    /// More groups: the new ones are empty and the last one takes focus. Fewer: the views of
    /// removed groups are appended, in order, to the last remaining group, and the focused
    /// view keeps focus.
    pub fn set_layout(&mut self, layout: Layout) {
        let n = layout.groups().max(1);
        let old = self.groups.len();
        if n > old {
            self.groups.extend((old..n).map(|_| Group::default()));
            self.active = n - 1;
        } else if n < old {
            let focused = self.active_tab().cloned();
            let removed: Vec<Group<T>> = self.groups.drain(n..).collect();
            let last = &mut self.groups[n - 1];
            last.selected.clear();
            for g in removed {
                last.tabs.extend(g.tabs);
                // pinned views stay pinned, behind the last group's own
                last.pinned.extend(g.pinned);
            }
            last.sort_pinned();
            if self.active >= n {
                self.active = n - 1;
                if let Some(i) = focused.and_then(|f| last.tabs.iter().position(|t| *t == f)) {
                    last.active = i;
                }
            }
        }
        self.layout = layout;
    }

    pub fn focus(&mut self, g: usize) {
        if g < self.groups.len() {
            self.active = g;
        }
    }

    /// `layout` from `Layout::split(g, …)`: an empty group comes in as `g + 1` and takes focus.
    pub fn split(&mut self, g: usize, layout: Layout) {
        let at = (g + 1).min(self.groups.len());
        self.groups.insert(at, Group::default());
        self.layout = layout;
        self.active = at;
    }

    /// `layout` and `target` from `Layout::close(g)`: group `g`'s views go to the end of the
    /// group that grew over it (`target`, numbered as after the close), its active view
    /// staying active there, and that group takes focus.
    pub fn close(&mut self, g: usize, layout: Layout, target: usize) {
        if g >= self.groups.len() || self.groups.len() < 2 {
            return;
        }
        let gone = self.groups.remove(g);
        let target = target.min(self.groups.len() - 1);
        let into = &mut self.groups[target];
        if let Some(active) = gone.tabs.get(gone.active).cloned() {
            into.selected.clear();
            into.tabs.extend(gone.tabs);
            into.pinned.extend(gone.pinned);
            into.sort_pinned();
            into.active = into.tabs.iter().position(|t| *t == active).unwrap_or(0);
        }
        self.layout = layout;
        self.active = target;
    }

    pub fn neighbour(&self, forward: bool) -> usize {
        let n = self.groups.len();
        if forward {
            (self.active + 1) % n
        } else {
            (self.active + n - 1) % n
        }
    }
}
