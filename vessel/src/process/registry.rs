use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
};
use voyage_protocol::process::{LocalAccessCredential, ProcessRegistration};

pub fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if !path.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "private directory must not be a symlink"
    );
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
        "private directory must be owned by current user with mode 0700"
    );
    Ok(())
}

pub fn lock(directory: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join("supervisor.lock"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe supervisor ownership lock"
    );
    ensure!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "another Vessel owns this directory"
    );
    Ok(file)
}

pub fn load(path: &Path) -> Result<ProcessRegistration> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() < 16384,
        "invalid private registration"
    );
    serde_json::from_reader(file).context("invalid process registration")
}

pub async fn save(directory: &Path, registration: &ProcessRegistration) -> Result<()> {
    let root = directory
        .parent()
        .and_then(Path::parent)
        .context("invalid session directory")?;
    super::database::save(root, registration).await?;
    publish(directory, registration)
}

pub fn publish(directory: &Path, registration: &ProcessRegistration) -> Result<()> {
    use std::io::Write;
    let bytes = serde_json::to_vec(registration)?;
    ensure!(bytes.len() < 16384, "process registration exceeds limit");
    let temporary = directory.join(format!("registration.{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(temporary, directory.join("registration.json"))?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

pub fn save_local_access(root: &Path, credential: &LocalAccessCredential) -> Result<()> {
    use std::io::Write;
    let bytes = serde_json::to_vec(credential)?;
    ensure!(bytes.len() < 4096, "local access credential exceeds limit");
    let temporary = root.join(format!(".process-http.{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(temporary, root.join("process-http.json"))?;
    File::open(root)?.sync_all()?;
    Ok(())
}

pub fn load_local_access(root: &Path) -> Result<LocalAccessCredential> {
    use std::io::Read;
    private_directory(root)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join("process-http.json"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() < 4096,
        "invalid local access credential"
    );
    let mut bytes = Vec::new();
    file.take(4096).read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes).context("invalid local access credential")
}

pub fn directory(root: &Path, session: uuid::Uuid) -> PathBuf {
    root.join("sessions").join(session.to_string())
}

/// Keep immutable lifecycle IDs across runtime incarnations and supervisor restarts.
pub async fn command_record(
    root: &Path,
    id: uuid::Uuid,
    command: &voyage_protocol::process::VesselCommand,
    reserve: bool,
) -> Result<bool> {
    super::database::command(root, "commands", id, serde_json::to_vec(command)?, reserve).await
}
