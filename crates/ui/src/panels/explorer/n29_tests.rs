use super::*;
use crate::workspace::ui_tests::{settle_until, window};
use gpui::{TestAppContext, VisualTestContext};

fn deep_tree(ws: &Entity<Workspace>, cx: &mut VisualTestContext, name: &str) -> PathBuf {
    let root = tests::temp(name).canonicalize().unwrap();
    let deep = root.join("a/b/c/d");
    std::fs::create_dir_all(&deep).unwrap();
    for i in 0..80 {
        std::fs::write(deep.join(format!("chapter-{i:02}.txt")), "source\n").unwrap();
    }
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(root.clone(), window, cx);
        for dir in deep.ancestors().take_while(|p| p.starts_with(&root)) {
            ws.sidebar.expand(dir.to_path_buf());
        }
        tests::poll_all(&mut ws.sidebar, std::slice::from_ref(&root));
        ws.sidebar.selected = Some(deep.clone());
        window.focus(&ws.sidebar_focus);
        cx.notify();
    });
    cx.run_until_parked();
    root
}

fn scroll_to(ws: &Entity<Workspace>, cx: &mut VisualTestContext, item: usize) {
    ws.update(cx, |ws, cx| {
        ws.sidebar.scroll.scroll_to_item_strict(item, gpui::ScrollStrategy::Top);
        cx.notify();
    });
    cx.run_until_parked();
}

#[gpui::test]
fn n29_natural_setting_reorders_cached_folders_and_files_and_refresh(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let root = tests::temp("orlo-n29-natural").canonicalize().unwrap();
    for name in ["chapter-2", "chapter-10"] {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    for name in ["part-2", "part-10", "part-02"] {
        std::fs::write(root.join(name), "").unwrap();
    }
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(root.clone(), window, cx);
        tests::poll_all(&mut ws.sidebar, std::slice::from_ref(&root));
    });
    cx.run_until_parked();
    let names = |ws: &mut Workspace| {
        ws.sidebar
            .rows()
            .iter()
            .skip(1)
            .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };
    ws.update(cx, |ws, _| assert_eq!(names(ws)[..2], ["chapter-10", "chapter-2"]));
    ws.update_in(cx, |ws, window, cx| {
        ws.apply_settings(r#"{"explorer_sort":"natural","lsp_enabled":false}"#, window, cx);
    });
    cx.run_until_parked();
    ws.update(cx, |ws, _| assert_eq!(names(ws), ["chapter-2", "chapter-10", "part-2", "part-02", "part-10"]));
    std::fs::write(root.join("part-3"), "").unwrap();
    ws.update(cx, |ws, cx| {
        ws.sidebar.refresh();
        tests::poll_all(&mut ws.sidebar, std::slice::from_ref(&root));
        assert_eq!(names(ws), ["chapter-2", "chapter-10", "part-2", "part-02", "part-3", "part-10"]);
        cx.notify();
    });
    ws.update_in(cx, |ws, window, cx| ws.apply_settings(r#"{"lsp_enabled":false}"#, window, cx));
    cx.run_until_parked();
    ws.update(cx, |ws, _| assert_eq!(names(ws)[..2], ["chapter-10", "chapter-2"]));
}

#[gpui::test]
fn n29_sticky_rows_use_real_pointer_keyboard_context_and_focus(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let root = deep_tree(&ws, cx, "orlo-n29-sticky-actions");
    scroll_to(&ws, cx, 25);
    assert!(cx.debug_bounds("explorer-sticky-0-0").is_none(), "off by default");
    ws.update_in(cx, |ws, window, cx| {
        ws.apply_settings(r#"{"explorer_sticky_ancestors":true,"lsp_enabled":false}"#, window, cx)
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("explorer-sticky-0-4").expect("deep ancestor pinned");
    cx.simulate_click(bounds.center(), gpui::Modifiers { platform: true, ..Default::default() });
    ws.read_with(cx, |ws, _| assert_eq!(ws.sidebar.marked, [root.join("a/b/c/d")]));
    cx.simulate_mouse_down(bounds.center(), gpui::MouseButton::Right, gpui::Modifiers::none());
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.sidebar.selected.as_deref(), Some(root.join("a/b/c/d").as_path()));
        assert_eq!(ws.context_menu.as_ref().unwrap().at, bounds.center());
    });
    cx.update(|window, cx| assert!(ws.read(cx).sidebar_focus.is_focused(window)));
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("down");
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.sidebar.selected.as_deref(), Some(root.join("a/b/c/d/chapter-00.txt").as_path()))
    });
    cx.run_until_parked();
    let selected = cx.debug_bounds("explorer-tree-0-5").expect("keyboard selection revealed");
    let last_pin = cx.debug_bounds("explorer-sticky-0-4");
    assert!(last_pin.is_none_or(|p| selected.top() >= p.bottom()));
    scroll_to(&ws, cx, 25);
    let bounds = cx.debug_bounds("explorer-sticky-0-4").unwrap();
    cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.sidebar.is_expanded(&root.join("a/b/c/d"))));
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.sidebar.selected.as_deref(), Some(root.join("a/b/c/d").as_path()));
    });
}

#[gpui::test]
fn n29_sticky_geometry_survives_fractional_scroll_resize_and_reveal(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let root = deep_tree(&ws, cx, "orlo-n29-sticky-geometry");
    ws.update_in(cx, |ws, window, cx| {
        ws.apply_settings(r#"{"explorer_sticky_ancestors":true,"lsp_enabled":false}"#, window, cx)
    });
    scroll_to(&ws, cx, 25);
    let first = cx.debug_bounds("explorer-sticky-0-0").unwrap();
    let deepest = cx.debug_bounds("explorer-sticky-0-4").unwrap();
    assert_eq!(deepest.top(), first.top() + px(4. * ROW_H));
    let before = ws.read_with(cx, |ws, _| ws.sidebar.scroll.0.borrow().base_handle.offset());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: deepest.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-11.))),
        ..Default::default()
    });
    cx.run_until_parked();
    let after = ws.read_with(cx, |ws, _| ws.sidebar.scroll.0.borrow().base_handle.offset());
    assert_eq!(after.y, before.y - px(11.), "wheel over pinned row scrolls the real list");
    assert_eq!(cx.debug_bounds("explorer-sticky-0-0").unwrap().top(), first.top(), "fractional scroll stays pinned");
    cx.simulate_resize(gpui::size(px(900.), px(170.)));
    ws.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let capacity = ws
        .read_with(cx, |ws, _| {
            (f32::from(ws.sidebar.scroll.0.borrow().last_item_size.unwrap().item.height) / ROW_H).floor() as usize
        })
        .saturating_sub(1);
    assert!(capacity > 0 && capacity < 5, "capacity after resize: {capacity}");
    // GPUI retains removed debug selectors; check the current last row and actual hit targets.
    let last = cx.debug_bounds("explorer-sticky-0-4").unwrap();
    assert_eq!(last.top(), first.top() + px((capacity - 1) as f32 * ROW_H));
    cx.simulate_click(
        gpui::point(last.center().x, first.top() + px(ROW_H / 2.)),
        gpui::Modifiers { platform: true, ..Default::default() },
    );
    let nearest = [root.clone(), root.join("a"), root.join("a/b"), root.join("a/b/c"), root.join("a/b/c/d")]
        [5 - capacity]
        .clone();
    ws.update(cx, |ws, _| {
        assert_eq!(ws.sidebar.marked, [nearest], "short viewport pins the nearest ancestors");
        ws.sidebar.marked.clear();
    });
    let pin = cx.debug_bounds("explorer-sticky-0-4").unwrap();
    cx.simulate_click(pin.center(), gpui::Modifiers { platform: true, ..Default::default() });
    ws.read_with(cx, |ws, _| assert_eq!(ws.sidebar.marked, [root.join("a/b/c/d")]));
    ws.update(cx, |ws, cx| {
        ws.sidebar.selected = Some(root.join("a/b/c/d/chapter-39.txt"));
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    let target = cx.debug_bounds("explorer-tree-0-45").unwrap();
    if let Some(pin) = cx.debug_bounds("explorer-sticky-0-4") {
        assert!(target.top() >= pin.bottom());
    }
    assert!(
        target.bottom()
            <= ws.read_with(cx, |ws, _| {
                let handle = ws.sidebar.scroll.0.borrow();
                first.top() + handle.last_item_size.unwrap().item.height
            })
    );
    cx.simulate_click(target.center(), gpui::Modifiers::none());
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| {
            ws.active_editor().unwrap().read(cx).doc(cx).path.as_deref()
                == Some(root.join("a/b/c/d/chapter-40.txt").as_path())
        })
    });
    cx.update(|window, cx| assert!(ws.read(cx).sidebar_focus.is_focused(window)));
}

#[gpui::test]
fn n29_sticky_reveal_crosses_from_deep_files_to_shallow_siblings(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let root = deep_tree(&ws, cx, "orlo-n29-reveal-shallow");
    for i in 0..20 {
        std::fs::write(root.join(format!("z-{i:02}.txt")), "shallow\n").unwrap();
    }
    ws.update_in(cx, |ws, window, cx| {
        ws.sidebar.refresh();
        tests::poll_all(&mut ws.sidebar, std::slice::from_ref(&root));
        ws.apply_settings(r#"{"explorer_sticky_ancestors":true,"lsp_enabled":false}"#, window, cx);
    });
    cx.simulate_resize(gpui::size(px(900.), px(170.)));
    cx.run_until_parked();
    ws.update(cx, |ws, cx| {
        ws.sidebar.selected = Some(root.join("a/b/c/d/chapter-79.txt"));
        ws.sidebar.reveal_row(84, 0);
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert_eq!(ws.sidebar.selected.as_deref(), Some(root.join("z-00.txt").as_path())));
    let target = cx.debug_bounds("explorer-tree-0-85").expect("shallow sibling revealed");
    let pin = cx.debug_bounds("explorer-sticky-0-4").expect("viewport still begins in deep subtree");
    assert!(target.top() >= pin.bottom(), "shallow target {target:?} must be below actual pins {pin:?}");
    cx.simulate_click(target.center(), gpui::Modifiers::none());
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| {
            ws.active_editor().unwrap().read(cx).doc(cx).path.as_deref() == Some(root.join("z-00.txt").as_path())
        })
    });
}

#[gpui::test]
fn n29_sticky_rows_keep_overlapping_root_identity_with_open_files(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let root = deep_tree(&ws, cx, "orlo-n29-sticky-roots");
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(root.join("a"), window, cx);
        ws.sidebar.show_open_files = true;
        ws.apply_settings(r#"{"explorer_sticky_ancestors":true,"lsp_enabled":false}"#, window, cx);
    });
    cx.run_until_parked();
    let (above, second) = ws.update(cx, |ws, cx| {
        let above = open_rows(ws, cx).len() + 1;
        let second = ws.sidebar.rows().iter().position(|r| r.root == 1).unwrap();
        assert_eq!(second, 85);
        (above, second)
    });
    scroll_to(&ws, cx, above + second + 20);
    assert!(cx.debug_bounds("explorer-sticky-0-0").is_none());
    let pin = cx.debug_bounds("explorer-sticky-1-85").expect("second root's own identity");
    cx.simulate_mouse_down(pin.center(), gpui::MouseButton::Right, gpui::Modifiers::none());
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.sidebar.selected.as_deref(), Some(root.join("a").as_path()));
        assert!(ws
            .context_menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .flatten()
            .any(|(label, _)| label == "Remove Folder from Project"));
    });
    cx.simulate_keystrokes("escape");
    scroll_to(&ws, cx, 0);
    ws.read_with(cx, |ws, _| assert_eq!(ws.sidebar.scroll.0.borrow().base_handle.offset().y, px(0.)));
    cx.simulate_click(pin.center(), gpui::Modifiers { platform: true, ..Default::default() });
    ws.read_with(cx, |ws, _| assert!(ws.sidebar.marked.is_empty(), "no stale pinned hitbox over OPEN FILES"));
    ws.read_with(cx, |ws, _| assert_eq!(ws.roots.len(), 2));
}
