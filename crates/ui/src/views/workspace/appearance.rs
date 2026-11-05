//! Themes and language definitions from the support folder, loaded at launch and again by
//! Reload Customizations.

use crate::chrome::theme;
use crate::{app::session, config::settings::Settings, languages::highlight};
use gpui::{App, Window, WindowAppearance};
use std::path::PathBuf;

pub fn load_customizations(settings: &Settings) -> Vec<String> {
    highlight::reload_customizations(&session::support_dir(), &settings.color_scheme, &settings.theme)
}

/// Open these folders in Finder to install or edit loose files and expanded packages.
pub fn customization_folder(kind: &str) -> std::io::Result<PathBuf> {
    if !matches!(kind, "Syntaxes" | "Themes" | "Packages") {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Unknown customization folder"));
    }
    let path = session::support_dir().join(kind);
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

/// Light or dark chrome, from the `theme` setting and the window's macOS appearance.
pub(crate) fn sync_theme(window: &Window, cx: &App) {
    let macos_dark = matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
    theme::set_dark(cx.global::<Settings>().dark(macos_dark));
}

/// A `font_face` that is not installed is replaced by `previous`; the alert text says so.
pub fn check_font(s: &mut Settings, previous: &str, cx: &App) -> Option<String> {
    if s.font_face == previous || cx.text_system().all_font_names().contains(&s.font_face) {
        return None;
    }
    let e = format!("The font \"{}\" is not installed. Orlo keeps using {previous}.", s.font_face);
    s.font_face = previous.into();
    Some(e)
}
