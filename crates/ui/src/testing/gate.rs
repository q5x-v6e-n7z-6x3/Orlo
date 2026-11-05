//! Deterministic pauses between real asynchronous work and UI admission/publication.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::task::{Poll, Waker};

#[derive(Default)]
struct Shared {
    arrived: Cell<usize>,
    open: Cell<bool>,
    waker: RefCell<Option<Waker>>,
}
thread_local! {
    static GATES: RefCell<HashMap<(&'static str, PathBuf), Rc<Shared>>> = RefCell::default();
}
pub(crate) struct Pause(Rc<Shared>, &'static str, PathBuf);
pub(crate) fn pause(point: &'static str, path: &Path) -> Pause {
    let shared = Rc::new(Shared::default());
    GATES.with(|g| g.borrow_mut().insert((point, path.to_path_buf()), shared.clone()));
    Pause(shared, point, path.to_path_buf())
}
impl Pause {
    pub(crate) fn arrived(&self) -> usize {
        self.0.arrived.get()
    }
    pub(crate) fn release(&self) {
        self.0.open.set(true);
        GATES.with(|g| g.borrow_mut().remove(&(self.1, self.2.clone())));
        if let Some(w) = self.0.waker.borrow_mut().take() {
            w.wake();
        }
    }
}
impl Drop for Pause {
    fn drop(&mut self) {
        self.release();
    }
}
pub(crate) fn holds(point: &'static str, path: &Path) -> bool {
    GATES.with(|g| g.borrow().contains_key(&(point, path.to_path_buf())))
}
pub(crate) async fn wait(point: &'static str, path: &Path) {
    let Some(shared) = GATES.with(|g| g.borrow().get(&(point, path.to_path_buf())).cloned()) else { return };
    shared.arrived.set(shared.arrived.get() + 1);
    std::future::poll_fn(|cx| match shared.open.get() {
        true => Poll::Ready(()),
        false => {
            *shared.waker.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        }
    })
    .await
}
