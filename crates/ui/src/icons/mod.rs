//! The chrome's icons: line drawings on a 24pt grid (assets/icons), compiled into the binary and
//! painted in the element's text colour, as Zed draws its icons.

use gpui::{px, svg, AssetSource, Result, SharedString, Styled, Svg};
use std::borrow::Cow;

macro_rules! icons {
    ($($name:ident => $file:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Icon {
            $($name,)*
        }

        impl Icon {
            pub fn path(self) -> &'static str {
                match self {
                    $(Icon::$name => concat!("icons/", $file, ".svg"),)*
                }
            }
        }

        fn bytes(path: &str) -> Option<&'static [u8]> {
            match path {
                $(concat!("icons/", $file, ".svg") => {
                    Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icons/", $file, ".svg")))
                })*
                _ => None,
            }
        }

        const PATHS: &[&str] = &[$(concat!("icons/", $file, ".svg"),)*];
    };
}

icons! {
    CaseSensitive => "case-sensitive",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    Close => "close",
    Collapse => "collapse",
    Ellipsis => "ellipsis",
    Error => "error",
    EyeOff => "eye-off",
    File => "file",
    FileCode => "file-code",
    FilePlus => "file-plus",
    FileText => "file-text",
    Folder => "folder",
    FolderOpen => "folder-open",
    FolderPlus => "folder-plus",
    GitBranch => "git-branch",
    GitCommit => "git-commit",
    GitPanel => "git-fork",
    Highlight => "highlight",
    InSelection => "in-selection",
    Layers => "layers",
    Minus => "minus",
    OpenExternal => "arrow-up-right",
    PanelLeft => "panel-left",
    Pilcrow => "pilcrow",
    Pin => "pin",
    Plus => "plus",
    PreserveCase => "preserve-case",
    Refresh => "refresh",
    Regex => "regex",
    Search => "search",
    Terminal => "terminal",
    Trash => "trash",
    Undo => "undo",
    Warning => "warning",
    WholeWord => "whole-word",
    Wrap => "wrap",
}

/// `icon` at `size` points; it takes the text colour of its element.
pub fn icon(icon: Icon, size: f32) -> Svg {
    svg().path(icon.path()).size(px(size)).flex_none()
}

/// Serves the icons to gpui (`Application::with_assets`).
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(bytes(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(PATHS.iter().filter(|p| p.starts_with(path)).map(|p| SharedString::from(*p)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_line_drawing_the_asset_source_serves() {
        assert!(!PATHS.is_empty());
        for path in PATHS {
            let svg = std::str::from_utf8(bytes(path).unwrap()).unwrap();
            assert!(svg.starts_with("<svg") && svg.contains("viewBox=\"0 0 24 24\""), "{path}");
        }
        assert_eq!(Assets.load(Icon::Close.path()).unwrap().as_deref(), bytes("icons/close.svg"));
        assert!(Assets.load("icons/missing.svg").unwrap().is_none());
        assert_eq!(Assets.list("icons/").unwrap().len(), PATHS.len());
    }
}
