//! Orlo's light and dark chrome and the "Mariana" colour scheme, as constants. The editor's
//! geometry and Mariana were sampled from Sublime Text build 4215; the chrome is Orlo's own.
pub mod assets;
pub use assets::{asset_matches, customization_files, language_folders};

use gpui::{rgb, rgba, Hsla};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock, RwLockReadGuard};
use syntect::highlighting::ThemeSettings;

#[derive(Default)]
struct Palette {
    editor: HashMap<u32, Hsla>,
    chrome: HashMap<&'static str, Hsla>,
    line_highlight: Option<Hsla>,
    selection_foreground: Option<Hsla>,
}

fn palette() -> &'static RwLock<Palette> {
    static PALETTE: OnceLock<RwLock<Palette>> = OnceLock::new();
    PALETTE.get_or_init(|| RwLock::new(Palette::default()))
}

fn read() -> RwLockReadGuard<'static, Palette> {
    palette().read().unwrap_or_else(|e| e.into_inner())
}

/// Replace part of the palette, then note whether anything is customized.
fn customize(change: impl FnOnce(&mut Palette)) {
    let mut p = palette().write().unwrap_or_else(|e| e.into_inner());
    change(&mut p);
    CUSTOM.store(!p.editor.is_empty() || !p.chrome.is_empty(), Ordering::Relaxed);
}

/// The default colors avoid locking on the editor's paint path.
static CUSTOM: AtomicBool = AtomicBool::new(false);

pub fn c(hex: u32) -> Hsla {
    if CUSTOM.load(Ordering::Relaxed) {
        if let Some(color) = read().editor.get(&hex) {
            return *color;
        }
    }
    rgb(hex).into()
}

/// For the `0xRRGGBBAA` constants.
pub fn ca(hex: u32) -> Hsla {
    rgba(hex).into()
}

/// A chrome colour, `0xRRGGBBAA`, in the light Default theme and in Default Dark.
#[derive(Clone, Copy)]
pub struct Ui(pub u32, pub u32, pub &'static str);

/// Which of the two the windows draw: one for the app, like the macOS appearance it follows.
static DARK: AtomicBool = AtomicBool::new(false);

pub fn set_dark(dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
}

pub fn ui(u: Ui) -> Hsla {
    if CUSTOM.load(Ordering::Relaxed) {
        let palette = read();
        if let Some(color) = palette.chrome.get(u.2) {
            return *color;
        }
        if u.2 == INACTIVE_EDITOR_BG.2 {
            if let Some(bg) = palette.editor.get(&EDITOR_BG) {
                return *bg;
            }
        }
    }
    ca(if DARK.load(Ordering::Relaxed) { u.1 } else { u.0 })
}

fn scheme_colors(settings: &ThemeSettings) -> HashMap<u32, Hsla> {
    [
        (EDITOR_BG, settings.background),
        (EDITOR_FG, settings.foreground),
        (CARET, settings.caret),
        (SELECTION, settings.selection),
        (SELECTION_BORDER, settings.selection_border),
        (GUTTER_FG, settings.gutter_foreground),
        (GUIDE, settings.guide),
        (FIND_CURRENT_BG, settings.find_highlight),
        (FIND_CURRENT_FG, settings.find_highlight_foreground),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|v| (key, syntect_color(v))))
    .collect()
}

fn syntect_color(c: syntect::highlighting::Color) -> Hsla {
    rgba(u32::from_be_bytes([c.r, c.g, c.b, c.a])).into()
}

pub fn apply_scheme(settings: &ThemeSettings) {
    customize(|p| {
        p.editor = scheme_colors(settings);
        p.editor.retain(|key, value| *value != Hsla::from(rgb(*key)));
        p.line_highlight = settings.line_highlight.map(syntect_color);
        p.selection_foreground = settings.selection_foreground.map(syntect_color);
    });
}

/// Mariana shares this RGB with its selection; custom schemes can set them separately.
pub fn line_highlight() -> Hsla {
    read().line_highlight.unwrap_or_else(|| rgb(LINE_HIGHLIGHT).into())
}

pub fn selection_foreground() -> Option<Hsla> {
    read().selection_foreground
}

pub fn appearance_names(support: &Path) -> Vec<String> {
    let mut names = vec!["auto".into(), "Default.sublime-theme".into(), "Default Dark.sublime-theme".into()];
    names.extend(
        assets::customization_files(support)
            .0
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "orlo-theme"))
            .filter_map(|p| p.strip_prefix(support).ok().map(|p| p.to_string_lossy().into_owned())),
    );
    names
}

/// Orlo appearance files contain a flat JSON object of chrome names to #RRGGBB[AA].
pub fn load_appearance(support: &Path, selected: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let colors = if matches!(selected, "auto" | "Default.sublime-theme" | "Default Dark.sublime-theme") {
        HashMap::new()
    } else {
        let found = assets::customization_files(support)
            .0
            .into_iter()
            .find(|p| p.extension().is_some_and(|e| e == "orlo-theme") && assets::asset_matches(support, p, selected));
        let parsed = found.ok_or_else(|| format!("Appearance {selected:?} was not found.")).and_then(|path| {
            (std::fs::read_to_string(&path).map_err(|e| e.to_string()))
                .and_then(|text| parse_appearance(&text))
                .map_err(|e| format!("{}: {e}", path.display()))
        });
        parsed.unwrap_or_else(|e| {
            errors.push(e);
            HashMap::new()
        })
    };
    customize(|p| p.chrome = colors);
    errors
}

fn parse_appearance(text: &str) -> Result<HashMap<&'static str, Hsla>, String> {
    let values: HashMap<String, String> =
        serde_json::from_str(&crate::settings::strip_json(text)).map_err(|e| e.to_string())?;

    let mut colors = HashMap::new();
    for (name, value) in values {
        let color = UI_COLORS
            .iter()
            .find(|color| color.2 == name)
            .ok_or_else(|| format!("Unknown appearance color {name:?}."))?;
        let hex = value
            .strip_prefix('#')
            .filter(|v| matches!(v.len(), 6 | 8) && v.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| format!("{name} must be #RRGGBB or #RRGGBBAA."))?;
        let rgb = u32::from_str_radix(hex, 16).map_err(|e| format!("{name}: {e}"))?;
        colors.insert(color.2, rgba(if hex.len() == 6 { (rgb << 8) | 255 } else { rgb }).into());
    }
    Ok(colors)
}

// Mariana (the editor), the same under both themes
pub const EDITOR_BG: u32 = 0x303841;
pub const EDITOR_FG: u32 = 0xd8dee9;
pub const GUTTER_FG: u32 = 0x848b95;
/// The caret line's number, on the `LINE_HIGHLIGHT` band.
pub const GUTTER_FG_CURRENT: u32 = 0xbfc5d0;
/// Only the gutter of the caret line is highlighted (Sublime's `highlight_gutter`).
pub const LINE_HIGHLIGHT: u32 = 0x4d5864;
pub const SELECTION: u32 = 0x4d5864;
pub const SELECTION_BORDER: u32 = 0x647382;
pub const CARET: u32 = 0xf9ae58;
/// The find panel's other matches are outlined; the current one is filled, its text dark.
pub const FIND_OUTLINE: u32 = 0x5fb4b4;
pub const FIND_CURRENT_BG: u32 = 0xfac761;
pub const FIND_CURRENT_FG: u32 = 0x333333;
/// The five highlight styles: Mariana's red, orange, green, blue and purple at a third, so the
/// text stays readable and a selection (opaque grey) and the find outline (teal line) differ.
pub const HIGHLIGHT_STYLES: [u32; 5] = [0xec5f6759, 0xf9915759, 0x99c79459, 0x6699cc66, 0xc594c566];
/// Their names in the menus.
pub const HIGHLIGHT_NAMES: [&str; 5] = ["Red", "Orange", "Green", "Blue", "Purple"];

/// A replacement as the results browser previews it (Mariana's green).
pub const REPLACE_PREVIEW_BG: u32 = 0x99c794;
pub const GUIDE: u32 = 0x49515a;
/// The overlay scroll bar's puck, white at 0.2.
pub const SCROLLBAR: u32 = 0xffffff33;
/// The shadow over the last 8pt of the text while lines run on past it: black at 0.15 at
/// the edge (Sublime's #353d45 to #2d343b on an inactive group).
pub const OVERFLOW_SHADOW: u32 = 0x00000026;
pub const OVERFLOW_SHADOW_W: f32 = 8.;
/// The minimap's view box while the mouse is over it.
pub const MINIMAP_VIEWPORT: u32 = 0x80808040;
/// The gutter's fold buttons (and the marker of folded text): white at 0.4 over a dark scheme, black
/// over a light one, 0.6 under the pointer; Sublime's #83888d and #acafb3 on Mariana
/// (`fold_button_control`, tinted black for `file_light`).
pub const FOLD_BUTTON: f32 = 0.4;
pub const FOLD_BUTTON_HOVER: f32 = 0.6;
/// A filler row's hatching, a side-by-side diff's padding: the fold buttons' tint at 0.08,
/// 1pt lines 6pt apart, faint enough to read as no text rather than as a mark.
pub const FILLER_HATCH: f32 = 0.08;
pub const FILLER_HATCH_LINE: f32 = 1.;
pub const FILLER_HATCH_GAP: f32 = 6.;

// Chrome: (light, dark), 0xRRGGBBAA. One cool neutral ramp (OKLCH hue 255, chroma under 0.015),
// one blue accent and three status hues, each light/dark pair measured: text 4.5:1 and icons and
// focus 3:1 against the surface it renders on, selected rows included.
macro_rules! ui_colors {
    ($($(#[$attr:meta])* pub const $name:ident: Ui = $color:expr;)*) => {
        $($(#[$attr])* pub const $name: Ui = $color;)*
        const UI_COLORS: &[Ui] = &[$($name),*];
    };
}

// The public roles and the names accepted in appearance files share one inventory.
ui_colors! {
    /// The editor background of groups without focus.
    pub const INACTIVE_EDITOR_BG: Ui = Ui(0x38424bff, 0x353d45ff, "inactive_editor_bg");
    /// Set in an appearance file, the tab strip's colour; otherwise `tab_strip_bg` derives it
    /// from the colour scheme.
    pub const TAB_BAR_BG: Ui = Ui(0x262d34ff, 0x262d34ff, "tab_bar_bg");
    /// The hairline between groups: the tab strip's colour unless set.
    pub const GROUP_BORDER: Ui = Ui(TAB_BAR_BG.0, TAB_BAR_BG.1, "group_border");
    pub const SIDEBAR_BG: Ui = Ui(0xf4f6f8ff, 0x1b2025ff, "sidebar_bg");
    /// Section headings (FOLDERS, OPEN FILES, a panel's sections).
    pub const SIDEBAR_HEADING: Ui = Ui(0x65696fff, 0x94999eff, "sidebar_heading");
    pub const SIDEBAR_FG: Ui = Ui(0x2a2e34ff, 0xcaced3ff, "sidebar_fg");
    pub const SIDEBAR_FG_SELECTED: Ui = Ui(0x0d1218ff, 0xf3f5f8ff, "sidebar_fg_selected");
    pub const SIDEBAR_FG_IGNORED: Ui = Ui(0x83868bff, 0x767b80ff, "sidebar_fg_ignored");
    /// Floating surfaces: the quick panel, context menus and tooltips.
    pub const OVERLAY_BG: Ui = Ui(0xfcfdffff, 0x272c32ff, "overlay_bg");
    /// The bottom panels and the status bar: the side bar's surface.
    pub const PANEL_BG: Ui = Ui(0xf4f6f8ff, 0x1b2025ff, "panel_bg");
    pub const QP_SELECTED_BG: Ui = Ui(0xe2ecfaff, 0x2e4056ff, "qp_selected_bg");
    pub const BUTTON_BG: Ui = Ui(0xffffffff, 0x2a3036ff, "button_bg");
    /// An option that is on (a find toggle, a shown panel's button).
    pub const BUTTON_SELECTED_BG: Ui = Ui(0xd5e5f9ff, 0x2f425bff, "button_selected_bg");
    pub const UI_FG: Ui = Ui(0x1f2329ff, 0xe0e3e7ff, "ui_fg");
    /// Secondary text: paths, counts, captions.
    pub const TEXT_MUTED: Ui = Ui(0x5f646aff, 0x9fa3a9ff, "text_muted");
    pub const QP_LABEL: Ui = Ui(0x25292fff, 0xd5d8dbff, "qp_label");
    pub const QP_LABEL_SELECTED: Ui = Ui(0x0d1218ff, 0xf5f7f9ff, "qp_label_selected");
    /// The letters a fuzzy query matched.
    pub const QP_MATCH: Ui = Ui(0x0064c4ff, 0x92c1fdff, "qp_match");
    pub const QP_PATH: Ui = Ui(0x65696fff, 0x94999eff, "qp_path");
    pub const QP_PATH_SELECTED: Ui = Ui(0x54585fff, 0xadb1b7ff, "qp_path_selected");

    // Hairlines and the states of controls
    /// Where two surfaces meet: the status bar's top, a dock's edge, a menu's edge.
    pub const BORDER: Ui = Ui(0xd8dbdfff, 0x2f353bff, "border");
    /// Lines inside a surface: separators, a header's underline.
    pub const BORDER_VARIANT: Ui = Ui(0xe4e6e9ff, 0x25292fff, "border_variant");
    pub const ELEMENT_HOVER: Ui = Ui(0xe7eaedff, 0x272c32ff, "element_hover");
    pub const ICON: Ui = Ui(0x494d54ff, 0xc1c4c9ff, "icon");
    pub const ICON_MUTED: Ui = Ui(0x767b80ff, 0x8b9095ff, "icon_muted");
    /// Focus and emphasis: the selected list row's edge, a primary button, the input tab.
    pub const ACCENT: Ui = Ui(0x1a73d5ff, 0x6fa7eeff, "accent");
    // Git status of a file in the side bars; each keeps 4.5:1 on the side bar and its selected row
    pub const GIT_MODIFIED: Ui = Ui(0x865901ff, 0xeec469ff, "git_modified");
    pub const GIT_ADDED: Ui = Ui(0x2f7134ff, 0x8ed690ff, "git_added");
    pub const GIT_CONFLICT: Ui = Ui(0xb32e2fff, 0xfe8a88ff, "git_conflict");

    // The find and quick panel inputs: inset a step below the surface
    pub const WIDGET_FG: Ui = Ui(0x171b20ff, 0xe6e8ebff, "widget_fg");
    pub const WIDGET_BG: Ui = Ui(0xffffffff, 0x14171dff, "widget_bg");
    pub const WIDGET_CARET: Ui = Ui(0x1a73d5ff, 0x6fa7eeff, "widget_caret");
    pub const WIDGET_SELECTION: Ui = Ui(0xc2daf9ff, 0x324e73ff, "widget_selection");
    /// Find in Files' Where input, and the placeholder text in it.
    pub const WHERE_BG: Ui = Ui(0xffffffff, 0x14171dff, "where_bg");
    pub const PLACEHOLDER_FG: Ui = Ui(0x83868bff, 0x7c8186ff, "placeholder_fg");

    // Ink on the tab strip, which sits on the colour scheme: `on_sheet` draws these white over a
    // dark scheme and black over a light one, at their alpha
    pub const TAB_ARROWS: Ui = Ui(0xffffff99, 0xffffff99, "tab_arrows");
    pub const TAB_HOVER_BG: Ui = Ui(0xffffff0f, 0xffffff0f, "tab_hover_bg");
    pub const TAB_ACTIVE_FG: Ui = Ui(0xfffffff2, 0xfffffff2, "tab_active_fg");
    pub const TAB_INACTIVE_FG: Ui = Ui(0xffffff9e, 0xffffff9e, "tab_inactive_fg");
    /// The close glyph and dirty dot: on the active tab, and on the others.
    pub const TAB_CLOSE: Ui = Ui(0xffffff99, 0xffffff99, "tab_close");
    pub const TAB_CLOSE_INACTIVE: Ui = Ui(0xffffff73, 0xffffff73, "tab_close_inactive");
    /// The line between two inactive tabs, and a dragged tab's edge.
    pub const TAB_DIVIDER: Ui = Ui(0xffffff1a, 0xffffff1a, "tab_divider");
    /// The close glyph and the strip's buttons under the mouse.
    pub const TAB_GLYPH_HOVER: Ui = Ui(0xffffffe6, 0xffffffe6, "tab_glyph_hover");
    /// The strip's buttons (tab list, new tab), and the wash behind one under the mouse.
    pub const NEW_TAB: Ui = Ui(0xffffff99, 0xffffff99, "new_tab");
    pub const TAB_BUTTON_HOVER_BG: Ui = Ui(0xffffff1a, 0xffffff1a, "tab_button_hover_bg");
    /// A group a dragged tab would drop into: the accent, faint.
    pub const DROP_TARGET_BG: Ui = Ui(0x1a73d526, 0x6fa7ee26, "drop_target_bg");
    /// Context menus' and tooltips' edge.
    pub const MENU_BORDER: Ui = Ui(0xd5d8dbff, 0x383e45ff, "menu_border");
    /// A panel's error line.
    pub const ERROR_FG: Ui = Ui(0xba2b28ff, 0xf7857dff, "error_fg");
    /// The status bar's icons.
    pub const SIDEBAR_TOGGLE: Ui = Ui(0x595e64ff, 0xa7abb1ff, "sidebar_toggle");
    /// Folder icons and disclosure chevrons.
    pub const SIDEBAR_ICON: Ui = Ui(0x82878cff, 0x888d92ff, "sidebar_icon");
    pub const SIDEBAR_FILE_GLYPH: Ui = Ui(0x82878cff, 0x888d92ff, "sidebar_file_glyph");
    /// The open file's row, and any row under the mouse: inset pills on the side bar.
    pub const SIDEBAR_SELECTED_BG: Ui = Ui(0xdeeaf9ff, 0x2a394dff, "sidebar_selected_bg");
    pub const SIDEBAR_HOVER_BG: Ui = Ui(0xeaedf1ff, 0x23282fff, "sidebar_hover_bg");
}

fn custom_chrome(u: Ui) -> Option<Hsla> {
    CUSTOM.load(Ordering::Relaxed).then(|| read().chrome.get(u.2).copied()).flatten()
}

/// A light colour scheme (black ink on it), else a dark one.
pub fn light_sheet() -> bool {
    c(EDITOR_BG).l > 0.5
}

/// The tab strip and the lines between groups: the colour scheme's background a step darker
/// (a dark scheme's by a fifth, a light one's by 0.06), so tabs read as the editor's own edge
/// under any scheme. An appearance file's `tab_bar_bg` or `group_border` wins.
pub fn tab_strip_bg(u: Ui) -> Hsla {
    custom_chrome(u).unwrap_or_else(|| {
        let bg = c(EDITOR_BG);
        Hsla { l: if bg.l > 0.5 { bg.l - 0.06 } else { bg.l * 0.8 }, ..bg }
    })
}

/// Ink on the tab strip: `u`'s alpha in white over a dark scheme, in black over a light one.
pub fn on_sheet(u: Ui) -> Hsla {
    custom_chrome(u)
        .unwrap_or_else(|| gpui::hsla(0., 0., if light_sheet() { 0. } else { 1. }, (u.0 & 0xff) as f32 / 255.))
}

/// The input tab's mark: the accent of the appearance that matches the scheme, so it keeps its
/// contrast on the sheet whichever appearance the chrome is in.
pub fn sheet_accent() -> Hsla {
    custom_chrome(ACCENT).unwrap_or_else(|| ca(if light_sheet() { ACCENT.0 } else { ACCENT.1 }))
}

/// A tab drag's insertion line and the group it would drop into.
pub const DROP_MARK: u32 = 0x5fb4b4;
/// The tab glyph of a file deleted (or renamed) on disk.
pub const DELETED_DOT: u32 = 0xd64e5c;

pub const EDITOR_FONT: &str = "Menlo";
pub const EDITOR_FONT_SIZE: f32 = 12.;
pub const UI_FONT: &str = ".SystemUIFont";
pub const UI_FONT_SIZE: f32 = 12.;

/// The tab strip; its tabs fill it, the active one in the sheet's colour.
pub const TAB_BAR_H: f32 = 36.;
pub const TAB_H: f32 = 36.;
pub const STATUS_BAR_H: f32 = 24.;
pub const SIDEBAR_W: f32 = 150.;
/// Dragging its edge keeps the side bar at least this wide …
pub const SIDEBAR_MIN_W: f32 = 50.;
/// … and leaves the editor at least this much of the window.
pub const SIDEBAR_MAX_MARGIN: f32 = 50.;
/// Width of the invisible strip that grabs a side bar edge or group divider.
pub const DIVIDER_HIT: f32 = 6.;
/// Dividers stop this close to their neighbours (Sublime squeezes groups to about 24pt).
pub const GROUP_MIN: f32 = 24.;
/// The gutter is this, plus a char per digit (two at least), plus `GUTTER_RIGHT`, wide; line
/// numbers end `GUTTER_RIGHT` short of its edge and text starts `TEXT_MARGIN` past it.
pub const GUTTER_LEFT: f32 = 15.5;
pub const GUTTER_RIGHT: f32 = 22.;
pub const TEXT_MARGIN: f32 = 4.;
/// Where line numbers end, short of the gutter's edge: Sublime's 22 to the digits' ink.
pub const GUTTER_NUMBER_RIGHT: f32 = 20.;
/// The minimap takes this share of the view right of the gutter; its rows are this share of
/// a line's height, its chars of a char's width.
pub const MINIMAP_SHARE: f32 = 0.117;
pub const MINIMAP_LINE: f32 = 2. / 15.;
pub const MINIMAP_CHAR: f32 = 1. / 8.;
/// The overlay scroll bar's puck: width, gap to the edge, shortest length; it hides this
/// long after the last scroll.
pub const SCROLL_PUCK_W: f32 = 7.;
pub const SCROLL_PUCK_RIGHT: f32 = 4.;
pub const SCROLL_PUCK_MIN: f32 = 24.;
pub const SCROLL_PUCK_SHOWN_MS: u64 = 1000;
/// The caret reaches this far above and below its line (`caret_extra_top/bottom`).
pub const CARET_EXTRA: f32 = 4.;
pub const GROUP_BORDER_W: f32 = 1.;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_chrome_colors_validate_every_key_before_apply() {
        let colors =
            parse_appearance("{\n// chrome colors\n\"sidebar_bg\":\"#123456\",\"ui_fg\":\"#abcdef80\",\n}").unwrap();
        assert_eq!(colors[SIDEBAR_BG.2], rgba(0x123456ff).into());
        assert_eq!(colors[UI_FG.2], rgba(0xabcdef80).into());
        assert!(parse_appearance(r##"{"sidebar_bg":"#12345"}"##).unwrap_err().contains("sidebar_bg"));
        assert!(parse_appearance(r##"{"sidebr_bg":"#123456"}"##).unwrap_err().contains("sidebr_bg"));
        assert!(parse_appearance(r##"{"sidebar_bg":5}"##).is_err());
    }

    #[test]
    fn chrome_roles_with_equal_default_colors_keep_independent_overrides() {
        let colors = parse_appearance(r##"{"tab_divider":"#ff0000","new_tab":"#00ff00"}"##).unwrap();
        assert_eq!(colors[TAB_DIVIDER.2], rgba(0xff0000ff).into());
        assert_eq!(colors[NEW_TAB.2], rgba(0x00ff00ff).into());
        let colors = parse_appearance(r##"{"group_border":"#ff0000","tab_bar_bg":"#00ff00"}"##).unwrap();
        assert_eq!(colors[GROUP_BORDER.2], rgba(0xff0000ff).into());
        assert_eq!(colors[TAB_BAR_BG.2], rgba(0x00ff00ff).into());
    }

    #[test]
    fn scheme_colors_cover_editor_caret_selection_and_background() {
        use syntect::highlighting::{Color, ThemeSettings};
        let red = Color { r: 255, g: 0, b: 0, a: 128 };
        let s = ThemeSettings {
            background: Some(red),
            foreground: Some(red),
            selection: Some(red),
            caret: Some(red),
            ..Default::default()
        };
        let colors = scheme_colors(&s);
        for key in [EDITOR_BG, EDITOR_FG, SELECTION, CARET] {
            assert_eq!(colors[&key], rgba(0xff000080).into());
        }
        assert!(scheme_colors(&ThemeSettings::default()).is_empty());
    }
}
