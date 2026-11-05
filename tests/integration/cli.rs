//! Process-level CLI exits use a disposable home and never need a native window.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run(args: &[&str], input: &[u8], name: &str) -> std::process::Output {
    run_within(args, input, name, Duration::from_secs(5))
}

/// `run`, for a mode that does real work before it exits (a debug build compiles grammars slowly).
fn run_within(args: &[&str], input: &[u8], name: &str, limit: Duration) -> std::process::Output {
    let home = std::env::temp_dir().join(format!("orlo-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_Orlo"))
        .args(args)
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let deadline = Instant::now() + limit;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("CLI did not exit without starting the app: {}", String::from_utf8_lossy(&output.stderr));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(!home.join("Library/Application Support/Orlo").exists(), "early CLI exit wrote application support");
    std::fs::remove_dir_all(home).unwrap();
    output
}

#[test]
fn help_and_version_exit_without_gui_or_application_support() {
    let help = run(&["--help"], &[], "help");
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout).unwrap().contains("--wait"));
    let version = run(&["--version"], &[], "version");
    assert!(version.status.success());
    assert_eq!(String::from_utf8(version.stdout).unwrap(), format!("Orlo {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn bad_options_and_malformed_stdin_exit_nonzero_before_gui() {
    let option = run(&["--unknown"], &[], "bad-option");
    assert!(!option.status.success());
    assert!(String::from_utf8(option.stderr).unwrap().contains("unknown option"));
    let stdin = run(&["-"], b"\xff", "bad-stdin");
    assert!(!stdin.status.success());
    assert!(String::from_utf8(stdin.stderr).unwrap().contains("UTF-8"));
}

#[test]
fn check_language_reports_each_folder_and_fails_on_any_error() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let broken = std::env::temp_dir().join(format!("orlo-cli-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&broken);
    std::fs::create_dir_all(broken.join("json-wasm/queries")).unwrap();
    for file in ["language.json", "grammar.wasm"] {
        std::fs::copy(root.join("tests/fixtures/languages/json-wasm").join(file), broken.join("json-wasm").join(file))
            .unwrap();
    }
    std::fs::write(broken.join("json-wasm/queries/highlights.scm"), "(no_such_node) @string\n").unwrap();
    std::fs::create_dir_all(broken.join("mismatch")).unwrap();
    std::fs::write(broken.join("mismatch/language.json"), r#"{"schema":1,"id":"other","name":"O","scope":"source.o"}"#)
        .unwrap();
    let (good, template) = (root.join("assets/languages/pony"), root.join("docs/templates/example-language"));
    let folders = [good.as_path(), template.as_path(), &broken.join("json-wasm"), &broken.join("mismatch")];
    let args: Vec<_> = ["--check-language"].into_iter().chain(folders.iter().map(|f| f.to_str().unwrap())).collect();
    let output = run_within(&args, &[], "check-language", Duration::from_secs(60));
    assert!(!output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "pony: ok\nexample-language: ok\n");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let lines: Vec<_> = stderr.lines().collect();
    assert_eq!(lines.len(), 2, "{stderr}");
    assert!(lines[0].starts_with(&format!("{}: line 1", broken.join("json-wasm/queries/highlights.scm").display())));
    assert!(lines[0].contains("no_such_node"), "{stderr}");
    assert!(lines[1].starts_with(&format!("{}: ", broken.join("mismatch/language.json").display())), "{stderr}");
    let good =
        run_within(&["--check-language", good.to_str().unwrap()], &[], "check-language-good", Duration::from_secs(60));
    assert!(good.status.success(), "{}", String::from_utf8_lossy(&good.stderr));
    assert_eq!(String::from_utf8(good.stdout).unwrap(), "pony: ok\n");
    std::fs::remove_dir_all(broken).unwrap();
    let help = run(&["--help"], &[], "check-language-help");
    assert!(String::from_utf8(help.stdout).unwrap().contains("--check-language folder"));
}
