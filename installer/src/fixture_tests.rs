//! Thread-local boundaries: no tests can invoke the real user service manager.
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::{cell::RefCell, collections::VecDeque, fs, path::PathBuf};
thread_local! {
    static ARGS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static ACQUIRE: RefCell<String> = RefCell::new(String::from("raise RuntimeError('unexpected source acquisition')"));
    static ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static CALLS: RefCell<VecDeque<(Vec<String>, Result<String, String>)>> = const { RefCell::new(VecDeque::new()) };
}
pub fn root() -> Option<PathBuf> {
    ROOT.with(|r| r.borrow().clone())
}
pub fn systemctl(args: &[&str]) -> anyhow::Result<String> {
    CALLS.with(|calls| {
        let (expected, result) = calls
            .borrow_mut()
            .pop_front()
            .expect("unexpected systemctl call; real manager is never invoked");
        assert_eq!(args, expected);
        result.map_err(anyhow::Error::msg)
    })
}
// Thread-local paths do not isolate the process descriptor table. A child spawned
// by another test can inherit a flock until exec, keeping it held after its owner
// drops it (O_CLOEXEC only takes effect at exec). Serialize fixtures that spawn
// children or exercise lock lifetimes, without retrying or weakening lock checks.
// Planning workers borrow the owning fixture's context, not this guard.
pub fn process_guard() -> std::sync::MutexGuard<'static, ()> {
    static PROCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    PROCESS.lock().unwrap_or_else(|error| error.into_inner())
}

pub struct Fixture {
    pub root: PathBuf,
    _process: std::sync::MutexGuard<'static, ()>,
}
impl Fixture {
    pub fn new() -> Self {
        // Check before locking so an accidentally nested fixture fails, not hangs.
        ROOT.with(|r| assert!(r.borrow().is_none()));
        let process = process_guard();
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::current_dir().unwrap().join(format!(
            ".installer-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        ROOT.with(|r| {
            assert!(r.borrow().is_none());
            *r.borrow_mut() = Some(root.clone());
        });
        Self {
            root,
            _process: process,
        }
    }
    pub fn call(&self, args: &[&str], response: &str) {
        CALLS.with(|c| {
            c.borrow_mut().push_back((
                args.iter().map(|s| (*s).into()).collect(),
                Ok(response.into()),
            ))
        });
    }
    pub fn fail(&self, args: &[&str], response: &str) {
        CALLS.with(|c| {
            c.borrow_mut().push_back((
                args.iter().map(|s| (*s).into()).collect(),
                Err(response.into()),
            ))
        });
    }
    pub fn query(&self, property: &str, value: &str) {
        self.call(
            &[
                "show",
                "voyage-vessel.service",
                "--value",
                "--property",
                property,
            ],
            value,
        );
    }
    pub fn effective(&self, loaded: bool) {
        self.call(
            &["show", "--value", "--property", "UnitPath"],
            self.root.join("units").to_str().unwrap(),
        );
        self.query("DropInPaths", "");
        self.query(
            "FragmentPath",
            if loaded {
                self.root
                    .join("units/voyage-vessel.service")
                    .to_str()
                    .unwrap()
                    .to_owned()
            } else {
                String::new()
            }
            .as_str(),
        );
        if loaded {
            self.query("KillMode", "process");
            self.query("ExecStop", "");
            self.query("ExecStopPost", "");
        }
    }
    pub fn script(&self, relative: &str, body: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    pub fn done(&self) {
        CALLS.with(|c| assert!(c.borrow().is_empty(), "unconsumed calls: {:?}", c.borrow()));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        ROOT.with(|r| *r.borrow_mut() = None);
        CALLS.with(|c| c.borrow_mut().clear());
        fs::remove_dir_all(&self.root).unwrap();
    }
}

pub fn acquire() -> String {
    ACQUIRE.with(|a| a.borrow().clone())
}
pub fn set_acquire(script: &str) {
    ACQUIRE.with(|a| *a.borrow_mut() = script.into());
}
pub fn release(f: &Fixture, name: &str, version: &str) -> PathBuf {
    use crate::install::release::{BINARIES, Binary, Manifest};
    let bin = f.root.join(name).join("bin");
    let mut binaries = std::collections::BTreeMap::new();
    for binary in BINARIES {
        let path = f.script(
            &format!("{name}/bin/{binary}"),
            &format!("printf '{binary} {version}\\n'"),
        );
        binaries.insert(
            binary.into(),
            Binary {
                sha256: crate::install::files::hash(&path).unwrap(),
            },
        );
    }
    let manifest = Manifest {
        schema_version: 1,
        version: version.into(),
        target: format!("linux-{}", std::env::consts::ARCH),
        binaries,
    };
    fs::write(
        bin.parent().unwrap().join("release.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    bin
}

pub fn executable(pid: u32) -> std::io::Result<PathBuf> {
    fs::read_link(
        root()
            .expect("process identity requires fixture")
            .join(format!("pid-{pid}")),
    )
}

pub fn arguments() -> Vec<String> {
    ARGS.with(|a| a.borrow().clone())
}
pub fn set_arguments(args: &[&str]) {
    ARGS.with(|a| *a.borrow_mut() = args.iter().map(|s| (*s).into()).collect());
}
pub struct Context {
    root: Option<PathBuf>,
    calls: VecDeque<(Vec<String>, Result<String, String>)>,
    acquire: String,
}
pub fn capture() -> Context {
    Context {
        root: root(),
        calls: CALLS.with(|c| std::mem::take(&mut *c.borrow_mut())),
        acquire: acquire(),
    }
}
pub fn enter(context: Context) {
    ROOT.with(|r| *r.borrow_mut() = context.root);
    CALLS.with(|c| *c.borrow_mut() = context.calls);
    set_acquire(&context.acquire);
}

pub fn required_root() -> anyhow::Result<Option<PathBuf>> {
    Ok(Some(root().ok_or_else(|| {
        anyhow::anyhow!("test requires isolated fixture; real HOME forbidden")
    })?))
}
