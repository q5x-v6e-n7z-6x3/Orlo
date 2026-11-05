//! Distraction-free mode: its own presentation from the `distraction_free` setting,
//! macOS full screen with it, and the window exactly as it was when it ends.

use super::*;
use crate::app::session::Chrome;

/// The mode while it is on: the window as it was before, and whether the mode put the window
/// in full screen (and so takes it out) and full screen has been seen since.
pub struct DistractionFreeMode {
    pub normal: Chrome,
    pub entered_full_screen: bool,
    pub saw_full_screen: bool,
}

impl Workspace {
    /// The parts of the window the mode hides or shows, as they are now.
    pub(crate) fn chrome(&self) -> Chrome {
        Chrome {
            sidebar: self.sidebar.visible,
            git_sidebar: self.git.sidebar_open,
            tabs: self.show_tabs,
            status_bar: self.show_status_bar,
            minimap: self.overrides.minimap,
            panel_hidden: self.panel_hidden,
        }
    }

    fn set_chrome(&mut self, c: Chrome, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.visible = c.sidebar;
        self.git.sidebar_open = c.git_sidebar;
        self.sidebar_anim = None;
        self.show_tabs = c.tabs;
        self.show_status_bar = c.status_bar;
        self.overrides.minimap = c.minimap;
        self.panel_hidden = c.panel_hidden;
        if c.panel_hidden {
            self.keys_out_of_panel(window, cx);
        }
    }

    /// View > Enter / Exit Distraction Free Mode (`ctrl-cmd-shift-f`).
    pub(crate) fn toggle_distraction_free(
        &mut self,
        _: &ToggleDistractionFree,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.distraction_free.is_some() {
            true => self.exit_distraction_free(true, window, cx),
            false => self.enter_distraction_free(None, window, cx),
        }
    }

    /// The mode on: what the setting shows (or `shown`, as a session left it), full screen if
    /// it says so. What the window showed before is kept to come back to.
    pub(crate) fn enter_distraction_free(
        &mut self,
        shown: Option<Chrome>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.distraction_free.is_some() {
            return;
        }
        let d = cx.global::<Settings>().distraction_free.clone();
        let normal = self.chrome();
        let shown = shown.unwrap_or(Chrome {
            sidebar: d.show_sidebar,
            git_sidebar: false,
            tabs: d.show_tabs,
            status_bar: d.show_status_bar,
            minimap: Some(d.show_minimap),
            panel_hidden: !d.show_panel,
        });
        self.set_chrome(shown, window, cx);
        let full = d.full_screen && !window.is_fullscreen();
        if full {
            window.toggle_fullscreen();
        }
        self.distraction_free =
            Some(DistractionFreeMode { normal, entered_full_screen: full, saw_full_screen: window.is_fullscreen() });
        self.apply_view(cx);
        self.focus_editor(window, cx);
        autosave(cx);
    }

    /// The mode off: the window as it was before it (what was toggled inside the mode is
    /// forgotten), out of the full screen the mode entered when `leave_full_screen`.
    pub(crate) fn exit_distraction_free(
        &mut self,
        leave_full_screen: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mode) = self.distraction_free.take() else {
            return;
        };
        self.set_chrome(mode.normal, window, cx);
        if leave_full_screen && mode.entered_full_screen && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        self.apply_view(cx);
        autosave(cx);
    }

    /// Leaving macOS full screen (the green button, `ctrl-cmd-f`) leaves the mode too, once
    /// full screen was on since it began.
    pub(crate) fn watch_full_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mode) = self.distraction_free.as_mut() else {
            return;
        };
        if window.is_fullscreen() {
            mode.saw_full_screen = true;
        } else if mode.saw_full_screen {
            self.exit_distraction_free(false, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::window;
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn exiting_the_mode_moves_focus_out_of_a_restored_hidden_terminal(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            cx.global_mut::<Settings>().distraction_free.full_screen = false;
            ws.show_terminal(&ShowTerminal, window, cx);
            ws.toggle_panel(&TogglePanel, window, cx);
            assert!(ws.panel_hidden);
            ws.toggle_distraction_free(&ToggleDistractionFree, window, cx);
            ws.show_terminal(&ShowTerminal, window, cx);
            assert!(ws.terminal.as_ref().unwrap().read(cx).focus_handle(cx).is_focused(window));
            ws.toggle_distraction_free(&ToggleDistractionFree, window, cx);
            assert!(ws.panel_hidden);
            assert!(ws.active_editor().unwrap().read(cx).is_focused(window));
            ws.close_terminal(&CloseTerminal, window, cx);
        });
    }

    #[gpui::test]
    fn entering_and_leaving_bring_the_window_back_as_it_was(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            let mut s = cx.global::<Settings>().clone();
            s.distraction_free.full_screen = false;
            cx.set_global(s);
            ws.show_tabs = false; // the user's own choice, kept through the mode
            ws.overrides.minimap = Some(true);
            let before = ws.chrome();
            for _ in 0..3 {
                ws.toggle_distraction_free(&ToggleDistractionFree, window, cx);
                assert!(ws.distraction_free.is_some());
                assert!(!ws.show_status_bar && !ws.sidebar.visible);
                let v = ws.effective_view(cx);
                assert!(!v.gutter && v.centered && v.wrap_width == 80 && !v.minimap);
                // shown inside the mode: for this stay only
                ws.toggle_status_bar(&ToggleStatusBar, window, cx);
                ws.toggle_minimap(&ToggleMinimap, window, cx);
                assert!(ws.show_status_bar);
                ws.toggle_distraction_free(&ToggleDistractionFree, window, cx);
                assert!(ws.distraction_free.is_none());
                assert_eq!(ws.chrome(), before, "as it was before");
                assert!(ws.effective_view(cx).gutter);
            }
        });
    }

    #[gpui::test]
    fn the_session_keeps_the_mode_and_the_window_before_it(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let st = ws.update_in(cx, |ws, window, cx| {
            let mut s = cx.global::<Settings>().clone();
            s.distraction_free.full_screen = false;
            cx.set_global(s);
            ws.toggle_distraction_free(&ToggleDistractionFree, window, cx);
            ws.toggle_tabs(&ToggleTabs, window, cx); // tabs shown inside the mode
            let mut texts = Texts::new();
            ws.capture(window, true, &mut texts, cx)
        });
        assert!(st.sidebar_visible && st.show_tabs && st.show_status_bar, "the window before");
        let shown = st.distraction_free.expect("the mode");
        assert!(shown.tabs && !shown.status_bar, "the mode as it was");
        // restored: the mode again as it was, and leaving it gives the window before
        ws.update_in(cx, |ws, window, cx| {
            ws.exit_distraction_free(true, window, cx);
            ws.restore(st, window, cx);
            assert!(ws.distraction_free.is_some());
            assert!(ws.show_tabs && !ws.show_status_bar);
            ws.toggle_distraction_free(&ToggleDistractionFree, window, cx);
            assert!(ws.show_tabs && ws.show_status_bar && ws.sidebar.visible);
        });
    }

    #[gpui::test]
    fn leaving_full_screen_leaves_the_mode(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.distraction_free = Some(DistractionFreeMode {
                normal: Chrome::default(),
                entered_full_screen: true,
                saw_full_screen: true,
            });
            // the test window is not in full screen: as if the green button had been pressed
            ws.watch_full_screen(window, cx);
            assert!(ws.distraction_free.is_none());
            assert_eq!(ws.chrome(), Chrome::default());
        });
    }
}
