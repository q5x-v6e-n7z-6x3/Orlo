/// The complete editing model, including its regression tests, as a large Rust fixture.
pub const EDITING_MODEL: &str = concat!(
    include_str!("../../editor/src/buffer/mod.rs"),
    include_str!("../../editor/src/commands/mod.rs"),
    include_str!("../../editor/src/commands/bookmarks.rs"),
    include_str!("../../editor/src/buffer/state.rs"),
    include_str!("../../editor/src/buffer/changes.rs"),
    include_str!("../../editor/src/commands/column_editing.rs"),
    include_str!("../../editor/src/commands/typing.rs"),
    include_str!("../../editor/src/history/mod.rs"),
    include_str!("../../editor/src/commands/line_editing.rs"),
    include_str!("../../editor/src/cursor/mod.rs"),
    include_str!("../../editor/src/buffer/persistence.rs"),
    include_str!("../../editor/src/search/model.rs"),
    include_str!("../../editor/src/selection/mod.rs"),
    include_str!("../../editor/src/commands/snippets.rs"),
    include_str!("../../editor/src/buffer/tests/changes.rs"),
    include_str!("../../editor/src/buffer/tests/columns.rs"),
    include_str!("../../editor/src/buffer/tests/drag_tests.rs"),
    include_str!("../../editor/src/buffer/tests/editing.rs"),
    include_str!("../../editor/src/buffer/tests/editor_feature_tests.rs"),
    include_str!("../../editor/src/buffer/tests/files.rs"),
    include_str!("../../editor/src/buffer/tests/lines.rs"),
    include_str!("../../editor/src/buffer/tests/marking_tests.rs"),
    include_str!("../../editor/src/buffer/tests/mod.rs"),
    include_str!("../../editor/src/buffer/tests/motion.rs"),
    include_str!("../../editor/src/buffer/tests/multi_caret.rs"),
    include_str!("../../editor/src/buffer/tests/search.rs"),
    include_str!("../../editor/src/buffer/tests/search_workflow_tests.rs"),
    include_str!("../../editor/src/buffer/tests/selections.rs"),
    include_str!("../../editor/src/buffer/tests/shared.rs"),
    include_str!("../../editor/src/buffer/tests/snippet_tests.rs"),
    include_str!("../../editor/src/buffer/tests/transforms.rs"),
    include_str!("../../editor/src/commands/transform.rs"),
);

/// Wait for a future on this thread.
pub(crate) fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
