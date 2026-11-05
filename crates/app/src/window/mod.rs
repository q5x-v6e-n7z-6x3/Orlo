use gpui::App;
use orlo_ui::{chrome, config, files, workspace};

pub fn restore(cx: &mut App, keymap_error: Option<String>) {
    // a broken settings file still starts Orlo, on the defaults, and says why
    let (mut settings, error) = config::settings::load();
    let font = workspace::check_font(&mut settings, chrome::theme::EDITOR_FONT, cx);
    let errors: Vec<String> =
        font.into_iter().chain(error).chain(keymap_error).chain(workspace::load_customizations(&settings)).collect();
    cx.set_global(settings);
    cx.set_global(workspace::SettingsDisk(files::watch::stamp(&config::settings::path())));
    let windows = workspace::restore_session(cx);
    workspace::save_on_quit(cx);
    workspace::quit_on_signals(cx);
    let window = windows[0];
    // with what compiling the user's grammars reports
    window
        .update(cx, |ws, window, cx| ws.report_customizations("Could not apply settings", errors, None, window, cx))
        .ok();
}
