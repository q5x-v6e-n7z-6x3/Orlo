//! Disposable native printing regression harness. Never submits a physical job.
//! Run with an empty output directory; inspect the generated PDFs with PDFKit.
use gpui::Application;
use orlo_ui::platform::{
    macos::printing::{self, Destination},
    printing::{Options, Snapshot},
};
use ropey::Rope;
use std::path::PathBuf;

fn main() {
    let directory = PathBuf::from(std::env::args().nth(1).expect("pass a disposable output directory"));
    std::fs::create_dir_all(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    orlo_utils::paths::TEST_SUPPORT.set(Some(directory.join("support")));
    orlo_ui::platform::macos::install_key_window_guard().unwrap();
    let window_mode = std::env::args().any(|arg| arg == "--window");
    Application::new().with_assets(orlo_ui::icons::Assets).run(move |cx| {
        if window_mode {
            cx.set_global(orlo_config::settings::Settings { lsp_enabled: false, ..Default::default() });
            cx.set_global(orlo_ui::workspace::SettingsDisk(None));
            orlo_ui::config::customization::bind_defaults(cx);
            orlo_ui::workspace::global_actions(cx);
            let windows = orlo_ui::workspace::restore_session(cx);
            orlo_ui::workspace::save_on_quit(cx);
            orlo_ui::workspace::quit_on_signals(cx);
            let fixture = directory.join("panel-source.txt");
            std::fs::write(&fixture, "Native panel cancellation — café 中 😀\tTAB\n".repeat(120)).unwrap();
            windows[0].update(cx, |ws, window, cx| ws.open_path(fixture, window, cx)).unwrap();
            cx.activate(true);
            return;
        }
        cx.spawn(async move |cx| {
            let mut text = "START α café 中 😀\tTABS\n".to_string();
            for i in 1..=240 {
                text.push_str(&format!("source {i:03}\tUnicode é 中 — {}\n", "wrap these words ".repeat(9)));
            }
            text.push_str("END document");
            let mut snapshot =
                Snapshot::new(Rope::from_str(&text), std::iter::once(0..20).collect(), "Orlo Print QA".into());
            snapshot.set_syntax("Rust".into());
            let offset = |marker: &str| text[..text.find(marker).unwrap()].chars().count();
            let mut excerpts = Snapshot::new(
                Rope::from_str(&text),
                vec![offset("source 020")..offset("source 101"), offset("source 180")..offset("source 221")],
                "Excerpts".into(),
            );
            excerpts.set_syntax("Rust".into());
            for (name, selection, options) in [
                ("document", false, Options::default()),
                ("selection", true, Options::default()),
                (
                    "numbered-large",
                    false,
                    Options { font_size: 16, tab_width: 8, line_numbers: true, ..Options::default() },
                ),
                ("empty", false, Options::default()),
                ("no-wrap", false, Options { wrap: false, ..Options::default() }),
                ("color", false, Options { syntax_colors: true, ..Options::default() }),
                ("selection-multi", true, Options { syntax_colors: true, line_numbers: true, ..Options::default() }),
            ] {
                let path = directory.join(format!("{name}.pdf"));
                assert!(!path.exists(), "refusing to overwrite {}", path.display());
                let source = if name == "empty" {
                    Snapshot::new(Rope::new(), vec![], "Empty".into()).prepare(false, options).unwrap()
                } else if name == "selection-multi" {
                    excerpts.prepare(true, options).unwrap()
                } else {
                    snapshot.prepare(selection, options).unwrap()
                };
                assert!(
                    printing::run_prepared("Orlo Print QA", &source, options, false, Destination::Pdf(&path)).unwrap()
                );
                assert!(std::fs::metadata(&path).unwrap().len() > 100);
                println!("native PDF saved: {}", path.display());
            }
            cx.update(|cx| cx.quit()).unwrap();
        })
        .detach();
    });
}
