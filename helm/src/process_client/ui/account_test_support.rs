//! Per-test synthetic storage and OS-browser effect boundary. Never changes HOME
//! or writes into the operator's account preferences/drafts during App tests.
use std::{cell::RefCell, path::PathBuf};
thread_local! {
    static ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static BROWSERS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}
pub(super) struct Fixture(pub tempfile::TempDir);
impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        ROOT.with(|r| *r.borrow_mut() = Some(dir.path().to_owned()));
        BROWSERS.with(|r| r.borrow_mut().clear());
        Self(dir)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        ROOT.with(|r| *r.borrow_mut() = None);
    }
}
pub(super) fn root(name: &str, default: PathBuf) -> PathBuf {
    ROOT.with(|r| r.borrow().as_ref().map(|r| r.join(name)).unwrap_or(default))
}
pub(super) fn browser(url: &str) -> anyhow::Result<()> {
    BROWSERS.with(|r| r.borrow_mut().push(url.to_owned()));
    Ok(())
}
pub(super) fn browsers() -> Vec<String> {
    BROWSERS.with(|r| r.borrow().clone())
}
