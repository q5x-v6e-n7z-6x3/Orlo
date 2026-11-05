//! Native window title derived from the active editor and project.

use crate::workspace::*;

impl Workspace {
    /// Sublime's: "file — project" (the project's name, else the first folder's), or the
    /// file alone with neither.
    pub(crate) fn window_title(&self, cx: &App) -> String {
        let folder = match &self.project {
            Some(p) => Some(p.name()),
            None => self.roots.first().map(|r| r.label()),
        };
        let Some(editor) = self.active_editor() else {
            return folder.unwrap_or_else(|| "Orlo".into());
        };
        let e = editor.read(cx);
        let file = format!(
            "{}{}{}",
            e.title(cx),
            if e.doc(cx).dirty { " •" } else { "" },
            if e.doc(cx).viewer_mode() { " [VIEWER]" } else { "" }
        );
        match folder {
            Some(folder) => format!("{file} — {folder}"),
            None => file,
        }
    }
}
