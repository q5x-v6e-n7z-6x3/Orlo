/// Install native application hooks before creating a window.
pub fn install() -> std::io::Result<()> {
    orlo_ui::platform::macos::install_key_window_guard()
}
