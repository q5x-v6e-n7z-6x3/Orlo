use gpui::App;
use orlo_ui::{config, workspace};

pub fn install(cx: &mut App) -> Option<String> {
    config::customization::bind_defaults(cx);
    let error = config::customization::install(cx).err();
    workspace::global_actions(cx);
    error
}
