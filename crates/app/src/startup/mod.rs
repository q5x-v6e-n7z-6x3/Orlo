use gpui::{App, Application};
use orlo_ui::{app, chrome, workspace};
use std::process::{Command, Stdio};

pub fn run() {
    orlo_utils::paths::use_user_support_dir();
    app::perf::start();
    let cwd = std::env::current_dir().unwrap_or_default();
    let request = match app::cli::parse(std::env::args_os().skip(1), &cwd, &mut std::io::stdin().lock()) {
        Ok(app::cli::Launch::Help) => {
            print!("{}", app::cli::HELP);
            return;
        }
        Ok(app::cli::Launch::Version) => {
            println!("Orlo {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Ok(app::cli::Launch::CheckLanguage(folders)) => std::process::exit(check_languages(&folders)),
        Ok(app::cli::Launch::Open(request)) => request,
        Err(error) => {
            eprintln!("Orlo: {error}");
            std::process::exit(2);
        }
    };
    let sock = app::handoff::socket_path();
    // held until Orlo exits
    let claim = if request.targets.is_empty() && !request.wait {
        app::handoff::claim(&sock, &[])
    } else {
        app::handoff::claim_request(&sock, &request)
    };
    let _lock = match claim {
        app::handoff::Claim::Primary(lock) if request.wait => {
            // The launcher can finish while the GUI retains unrelated restored windows.
            // A competing launch may win this gap; the same socket still receives our request.
            drop(lock);
            use std::os::unix::process::CommandExt;
            let started = std::env::current_exe().and_then(|exe| {
                Command::new(exe)
                    // The GUI must not keep a caller's captured output pipes open after --wait ends.
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .process_group(0)
                    .spawn()
            });
            if let Err(error) = started {
                fail(format!("could not start the application: {error}"));
            }
            if let Err(error) = app::handoff::send_request(&sock, &request) {
                fail(error);
            }
            return;
        }
        app::handoff::Claim::Primary(lock) => lock,
        app::handoff::Claim::Secondary => return,
        app::handoff::Claim::Failed(e) => fail(e),
    };
    // gpui registers its Cocoa classes before main; install before any window opens.
    if let Err(error) = crate::platform::install() {
        fail(format!("could not install the native window guard: {error}"));
    }
    let inbox = app::handoff::Inbox::default();
    app::handoff::listen(&sock, inbox.clone());
    let app = Application::new().with_assets(chrome::icons::Assets);
    let opened = inbox.clone();
    app.on_open_urls(move |urls| opened.push(urls.iter().filter_map(|u| app::handoff::file_url_path(u)).collect()));
    let initial = (!request.targets.is_empty()).then(|| app::handoff::Delivery::local(request));
    let initial_result = initial.as_ref().map(app::handoff::Delivery::completion);
    app.run(move |cx: &mut App| {
        cx.activate(true);
        // closing the last window would otherwise leave a menu bar that cannot quit
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let keymap_error = crate::actions::install(cx);
        crate::window::restore(cx, keymap_error);
        // files from Finder, the Dock and later `Orlo` launches
        cx.spawn(async move |cx| loop {
            let mut delivery = inbox.next_delivery().await;
            if cx
                .update(|cx| {
                    // This gate, opening the paths and acknowledging them share one UI update.
                    // Quit cannot snapshot between them and lose an acknowledged request.
                    if app::handoff::accepting() {
                        if delivery.request.is_some() {
                            workspace::open_request(delivery, cx);
                            return;
                        }
                        let paths = std::mem::take(&mut delivery.paths);
                        if workspace::open_paths(paths, cx) {
                            delivery.accept();
                        }
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
        if let Some(delivery) = initial {
            workspace::open_request(delivery, cx);
        }
    });
    if let Some(result) = initial_result {
        match result.outcome() {
            Some(Ok(())) => {}
            Some(Err(error)) => fail(error),
            None => fail("the application closed before confirming the requested files"),
        }
    }
}

/// Load and activate each language folder as Orlo would: `<id>: ok` on stdout for one that
/// works, its errors on stderr. The exit status: 1 if any failed.
fn check_languages(folders: &[std::path::PathBuf]) -> i32 {
    use orlo_ui::languages::{registry, treesitter};
    let mut failed = false;
    for folder in folders {
        match registry::LanguageDef::load(folder, registry::Origin::User)
            .map_err(|e| vec![e])
            .and_then(|def| treesitter::activate(&def).map(|()| def))
        {
            Ok(def) => println!("{}: ok", def.id()),
            Err(errors) => {
                failed = true;
                errors.iter().for_each(|e| eprintln!("{e}"));
            }
        }
    }
    failed as i32
}

/// Reports `error` on stderr and exits unsuccessfully.
fn fail(error: impl std::fmt::Display) -> ! {
    eprintln!("Orlo: {error}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn repo(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative)
    }

    #[test]
    fn bundled_languages_and_the_template_check_clean() {
        let mut folders: Vec<_> = std::fs::read_dir(repo("assets/languages"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        folders.push(repo("docs/templates/example-language"));
        assert!(folders.len() > 24);
        assert_eq!(super::check_languages(&folders), 0);
    }

    /// Its `queries/highlights.scm` compiles once a `tree_sitter` block names it, as the README says.
    #[test]
    fn the_template_highlights_query_compiles() {
        let folder =
            std::env::temp_dir().join(format!("orlo-template-{}", std::process::id())).join("example-language");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("queries")).unwrap();
        let template = repo("docs/templates/example-language");
        for file in ["language.json", "Example.sublime-syntax", "queries/highlights.scm"] {
            std::fs::copy(template.join(file), folder.join(file)).unwrap();
        }
        let mut manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(folder.join("language.json")).unwrap()).unwrap();
        manifest["tree_sitter"] =
            serde_json::json!({"grammar": "builtin:bash", "highlights": ["queries/highlights.scm"]});
        std::fs::write(folder.join("language.json"), manifest.to_string()).unwrap();
        assert_eq!(super::check_languages(std::slice::from_ref(&folder)), 0);
        std::fs::write(folder.join("queries/highlights.scm"), "(no_such_node) @string\n").unwrap();
        assert_eq!(super::check_languages(std::slice::from_ref(&folder)), 1);
        std::fs::remove_dir_all(folder.parent().unwrap()).unwrap();
    }
}
