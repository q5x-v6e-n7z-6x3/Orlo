//! The bottom panel area: one panel at a time, spanning the window under the side bar and the
//! groups, as Sublime places it. Each kind of panel is a variant of `Panel`.

use super::*;

pub enum Panel {
    Find(FindPanel),
    FindInFiles(FindInFilesPanel),
    Form(FormPanel),
    /// The results browser.
    Results(super::results_panel::ResultsPanel),
    /// The regex playground.
    Playground(super::playground::PlaygroundPanel),
    Terminal(Entity<crate::integrations::terminal::Terminal>),
    Changes(super::git_ui::ChangePanel),
    Comparison(super::comparison::ComparisonPanel),
    History(Box<super::history::HistoryPanel>),
    Rescue(Box<super::discarded::RescuePanel>),
}

impl Workspace {
    pub fn find_panel(&self) -> Option<&FindPanel> {
        match &self.panel {
            Some(Panel::Find(p)) => Some(p),
            _ => None,
        }
    }

    pub fn find_panel_mut(&mut self) -> Option<&mut FindPanel> {
        match &mut self.panel {
            Some(Panel::Find(p)) => Some(p),
            _ => None,
        }
    }

    pub fn fif_panel(&self) -> Option<&FindInFilesPanel> {
        match &self.panel {
            Some(Panel::FindInFiles(p)) => Some(p),
            _ => None,
        }
    }

    pub fn fif_panel_mut(&mut self) -> Option<&mut FindInFilesPanel> {
        match &mut self.panel {
            Some(Panel::FindInFiles(p)) => Some(p),
            _ => None,
        }
    }

    /// The single-line field of the open panel that has the keys, if any.
    pub(crate) fn focused_input(&self, window: &Window, cx: &App) -> Option<Entity<Editor>> {
        let inputs = match self.panel.as_ref()? {
            Panel::Find(p) => vec![&p.query, &p.replace],
            Panel::FindInFiles(p) => p.fields().to_vec(),
            Panel::Form(p) => p.inputs().collect(),
            Panel::Results(p) => vec![&p.filter],
            Panel::Playground(p) => vec![&p.pattern, &p.replace],
            Panel::Changes(p) => vec![&p.viewer],
            Panel::Comparison(p) => p.viewers(),
            Panel::History(p) => vec![&p.viewer],
            Panel::Rescue(p) => vec![&p.viewer],
            Panel::Terminal(_) => return None,
        };
        inputs.into_iter().find(|e| e.read(cx).is_focused(window)).cloned()
    }

    /// Replaces whatever panel is showing.
    pub(crate) fn show_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        self.cancel_comparison();
        self.remember_find(cx);
        self.remember_result_browser(cx);
        self.panel = Some(panel);
        self.reveal_panel(cx);
    }

    pub(crate) fn reveal_panel(&mut self, cx: &mut Context<Self>) {
        self.invalidate_restoring_active();
        self.invalidate_navigation();
        // a panel asked for is shown, whatever the last one was
        (self.panel_hidden, self.panel_collapsed) = (false, false);
        self.refresh_menus(cx);
        cx.notify();
    }

    pub(crate) fn render_panel(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match self.panel.as_ref()? {
            Panel::Find(p) => p.render(cx),
            Panel::FindInFiles(p) => p.render(cx),
            Panel::Results(_) => self.render_results_panel(cx),
            Panel::Playground(_) => self.render_playground(cx),
            Panel::Form(p) => p.render(cx),
            Panel::Terminal(_) => self.render_terminal_panel(cx),
            Panel::Changes(_) => self.render_change_panel(cx),
            Panel::Comparison(_) => self.render_comparison_panel(cx),
            Panel::History(_) => self.render_file_history(cx),
            Panel::Rescue(_) => self.render_rescue(cx),
        })
    }
}
