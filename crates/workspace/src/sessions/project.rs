//! A named project's private window state, stored in its `.orlo-workspace` file.

use super::WindowState;
use orlo_project::project::{absolute, relative};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A `.orlo-workspace` file: the project it belongs to (relative to the workspace file) and
/// one window as the session keeps windows, unsaved text inline.
#[derive(Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(default)]
pub struct WorkspaceFile {
    pub orlo_workspace: u32,
    pub project: String,
    pub window: WindowState,
}

impl WorkspaceFile {
    /// The project file this workspace names, made absolute.
    pub fn project_path(&self, workspace: &Path) -> PathBuf {
        absolute(&self.project, workspace.parent().unwrap_or(Path::new("/")))
    }

    /// Read `path`: a file that is not a workspace is an error, so a window never swaps its
    /// state for nothing.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if v.get("orlo_workspace").and_then(Value::as_u64) != Some(1) {
            return Err(format!("{} is not an Orlo workspace", path.display()));
        }
        let mut file: Self = serde_json::from_value(v).map_err(|e| format!("{}: {e}", path.display()))?;
        file.window.sanitize();
        // a workspace file comes with a folder: its drafts may be only of files in that folder
        // (through links too), or Save All would write what it brought over any of the user's
        let dir = path.parent().unwrap_or(Path::new("/"));
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let inside = |p: &Path| {
            let real = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            !p.components().any(|c| c == std::path::Component::ParentDir) && real.starts_with(&dir)
        };
        let views = file.window.groups.iter_mut().flat_map(|g| g.views.iter_mut());
        for v in views.filter(|v| v.path.is_absolute() && !inside(&v.path)) {
            v.unsaved = None;
        }
        Ok(file)
    }

    /// For `project`, written to a temporary file renamed into place, readable by its owner
    /// alone: it may hold unsaved text.
    pub fn save(path: &Path, project: &Path, window: WindowState) -> std::io::Result<()> {
        let dir = path.parent().unwrap_or(Path::new("/"));
        let f = Self { orlo_workspace: 1, project: relative(project, dir), window };
        let text = serde_json::to_string_pretty(&f).map_err(std::io::Error::other)?;
        orlo_project::io::write_atomic_private(path, &text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_received_workspace_file_keeps_drafts_only_of_its_own_folders_files() {
        use crate::sessions::{GroupState, Unsaved, ViewState};
        let dir = temp("orlo-workspace-drafts").canonicalize().unwrap();
        let outside = temp("orlo-workspace-drafts-outside").canonicalize().unwrap();
        std::fs::write(outside.join("rc"), "mine").unwrap();
        std::os::unix::fs::symlink(outside.join("rc"), dir.join("linked")).unwrap();
        let draft = |path: PathBuf| ViewState {
            path,
            unsaved: Some(Unsaved { text: "brought".into(), dirty: true, ..Default::default() }),
            ..Default::default()
        };
        let views = ["own.txt", "linked", "../orlo-workspace-drafts-outside/rc"].map(|n| draft(dir.join(n)));
        let views = views.into_iter().chain([draft(outside.join("rc"))]).collect();
        let window = WindowState { groups: vec![GroupState { views, active: 0 }], ..Default::default() };
        let path = dir.join("w.orlo-workspace");
        WorkspaceFile::save(&path, &dir.join("p.sublime-project"), window).unwrap();
        let back = WorkspaceFile::load(&path).unwrap();
        let kept: Vec<bool> = back.window.groups[0].views.iter().map(|v| v.unsaved.is_some()).collect();
        assert_eq!(kept, [true, false, false, false]);
    }

    #[test]
    fn a_received_workspace_file_cannot_hold_a_layout_a_window_cannot_draw() {
        let dir = std::env::temp_dir().join(format!("orlo-workspace-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("X.orlo-workspace");
        let text = r#"{"orlo_workspace":1,"project":"X.sublime-project","window":{"font_size":0,"layout":{"cols":[0,1],"rows":[0,1],"cells":[[0,0,9,9]]}}}"#;
        std::fs::write(&path, text).unwrap();
        let file = WorkspaceFile::load(&path).unwrap();
        assert!(file.window.layout.is_valid());
        assert!(file.window.font_size.is_none_or(|f| f >= 6.));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A default window's workspace at `path`, for `p.sublime-project` beside it.
    fn save_blank(path: &Path) -> std::io::Result<()> {
        let project = path.with_file_name("p.sublime-project");
        WorkspaceFile::save(path, &project, WindowState::default())
    }

    #[test]
    fn a_workspace_round_trips_and_names_its_project() {
        let dir = temp("orlo-workspace-file");
        let ws = dir.join("w.orlo-workspace");
        let window = WindowState { sidebar_width: 222., ..Default::default() };
        WorkspaceFile::save(&ws, &dir.join("p.sublime-project"), window).unwrap();
        let back = WorkspaceFile::load(&ws).unwrap();
        assert_eq!(back.project, "p.sublime-project");
        assert_eq!(back.project_path(&ws), dir.join("p.sublime-project"));
        assert_eq!(back.window.sidebar_width, 222.);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&ws).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::write(&ws, "{\"window\": {}}").unwrap();
        assert!(WorkspaceFile::load(&ws).is_err(), "not a workspace");
    }

    #[test]
    fn a_workspace_does_not_write_in_place_when_private_staging_is_unavailable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("orlo-workspace-private-staging");
        let path = dir.join("w.orlo-workspace");
        std::fs::write(&path, "original workspace").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = save_blank(&path);
        let contents = std::fs::read_to_string(&path).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err());
        assert_eq!(contents, "original workspace");
    }

    #[test]
    fn replacing_a_workspace_keeps_drafts_private_and_preserves_the_original_on_failure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("orlo-workspace-private-replacement");
        let path = dir.join("w.orlo-workspace");
        std::fs::write(&path, "original workspace").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        orlo_project::io::FAULT.set(Some(("write", std::io::ErrorKind::Other)));
        let result = save_blank(&path);
        orlo_project::io::FAULT.set(None);
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original workspace");
        save_blank(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }
}
