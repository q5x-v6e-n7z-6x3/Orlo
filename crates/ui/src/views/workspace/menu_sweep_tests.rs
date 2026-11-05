//! Every menu item, chosen in a window with a Rust file open and a word selected: none may
//! panic or leave the window without a workspace.

use super::ui_tests::{settle, temp, window};
use super::*;
use crate::editing::text::Selection;
use gpui::{MenuItem, TestAppContext};

/// What leaves the test app (Finder, the browser, the Trash, the process), and what gpui's test
/// platform cannot do: hide, minimise, zoom, close its only window, or show a file dialog.
const SKIP: [&str; 26] = [
    "Orlo/Quit Orlo",
    "Orlo/Hide Orlo",
    "Orlo/Hide Others",
    "Orlo/Show All",
    "Orlo/Customizations/Open Themes Folder",
    "Orlo/Customizations/Open Syntaxes Folder",
    "File/Open…",
    "File/Close Window",
    "Edit/Convert/Hash File…",
    "Edit/Transform/Apply Substitution Table…",
    "Find/Saved Searches/Import Saved Searches…",
    "Tools/Install Package…",
    "Tools/Compare/Compare Files…",
    "Tools/Compare/Compare Folders…",
    "Project/Open Project…",
    "Project/Add Folder to Project…",
    "Project/Side Bar Item/Reveal in Finder",
    "Project/Side Bar Item/Move to Trash",
    "Window/Minimize",
    "Window/Zoom",
    "Help/Orlo Help",
    "Help/Local Alpha Guide",
    "Help/Customization and Automation",
    "Help/Languages",
    "Help/Sublime Text Documentation",
    "Help/Report an Issue",
];

fn leaves(items: Vec<MenuItem>, path: &str, out: &mut Vec<(String, Box<dyn Action>)>) {
    for item in items {
        match item {
            MenuItem::Action { name, action, .. } => out.push((format!("{path}/{name}"), action)),
            MenuItem::Submenu(m) => leaves(m.items, &format!("{path}/{}", m.name), out),
            MenuItem::Separator | MenuItem::SystemMenu(_) => {}
        }
    }
}

const SOURCE: &str = "fn main() {\n    let greeting = \"hello\";\n    for i in 0..3 {\n        println!(\"{greeting} {i}\");\n    }\n}\n\nstruct Point { x: i32, y: i32 }\n";

#[gpui::test]
#[ignore = "about ten minutes; run with --ignored to choose every menu item"]
fn every_menu_item_runs_without_panicking(cx: &mut TestAppContext) {
    let dir = temp(&format!("orlo-menu-sweep-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), SOURCE).unwrap();
    std::fs::write(dir.join("notes.txt"), "alpha\nbeta\nalpha beta\n").unwrap();
    let start: usize = std::env::var("ORLO_SWEEP_START").map_or(0, |s| s.parse().unwrap());
    let count = {
        let (ws, vcx) = window(cx);
        let mut all = Vec::new();
        ws.update(vcx, |ws, cx| {
            for m in ws.menus(cx) {
                leaves(m.items, &m.name, &mut all)
            }
        });
        all.len()
    };
    for i in start..count {
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            ws.add_root(dir.clone(), window, cx);
            ws.open_path(dir.join("src/main.rs"), window, cx);
        });
        settle(vcx);
        let mut all = Vec::new();
        ws.update_in(vcx, |ws, window, cx| {
            let e = ws.active_editor().expect("main.rs is open");
            window.focus(&e.read(cx).focus_handle(cx));
            ws.with_active(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::new(20, 28)]));
            for m in ws.menus(cx) {
                leaves(m.items, &m.name, &mut all)
            }
        });
        let (name, action) = all.swap_remove(i);
        if SKIP.contains(&name.as_str()) {
            continue;
        }
        eprintln!("SWEEP {i} {name}");
        vcx.update(|window, cx| window.dispatch_action(action, cx));
        settle(vcx);
        // Escape whatever opened, as a user would
        vcx.simulate_keystrokes("escape escape");
        settle(vcx);
    }
    eprintln!("SWEEP DONE {count}");
}

/// The bar's order, and no command twice in one menu: a second label for the same action (Find's
/// old "Incremental Find" ran Find…) only makes a menu longer to scan.
#[gpui::test]
fn menus_are_in_order_and_no_menu_repeats_a_command(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    ws.update(vcx, |ws, cx| {
        let menus = ws.menus(cx);
        let names: Vec<_> = menus.iter().map(|m| m.name.to_string()).collect();
        let order = ["Orlo", "File", "Edit", "Selection", "Find", "View", "Goto", "Terminal", "Git", "Tools"];
        assert_eq!(names, [&order[..], &["Project", "Window", "Help"]].concat());
        for m in menus {
            let name = m.name.to_string();
            let mut all = Vec::new();
            leaves(m.items, &name, &mut all);
            for (i, (label, action)) in all.iter().enumerate() {
                if let Some((first, _)) = all[..i].iter().find(|(_, earlier)| earlier.partial_eq(action.as_ref())) {
                    panic!("{label} repeats {first}");
                }
            }
        }
    });
}

#[test]
fn help_guides_open_on_the_main_branch() {
    let repo = "https://github.com/q5x-v6e-n7z-6x3/Orlo";
    assert_eq!(super::view::guide_url(""), repo);
    assert_eq!(
        super::view::guide_url("docs/user-guide/languages.md"),
        format!("{repo}/blob/main/docs/user-guide/languages.md")
    );
    assert_eq!(super::view::guide_url("issues/new"), format!("{repo}/issues/new"));
}
