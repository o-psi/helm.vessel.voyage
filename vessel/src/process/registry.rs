use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
};
use voyage_protocol::process::ProcessRegistration;

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
    ensure!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "another Vessel owns this directory"
    );
    Ok(file)
}

pub fn load(path: &Path) -> Result<ProcessRegistration> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() < 16384,
        "invalid private registration"
    );
    serde_json::from_reader(file).context("invalid process registration")
}

pub fn save(directory: &Path, registration: &ProcessRegistration) -> Result<()> {
    use std::io::Write;
    let temporary = directory.join(format!("registration.{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(registration)?)?;
    file.sync_all()?;
    fs::rename(temporary, directory.join("registration.json"))?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

pub fn directory(root: &Path, session: uuid::Uuid) -> PathBuf {
    root.join("sessions").join(session.to_string())
}

pub fn secure_socket(path: &Path) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Keep immutable lifecycle IDs across runtime incarnations and supervisor restarts.
pub fn command_record(
    root: &Path,
    id: uuid::Uuid,
    command: &voyage_protocol::process::VesselCommand,
    reserve: bool,
) -> Result<bool> {
    use std::io::{Read, Write};
    let directory = root.join("commands");
    private_directory(&directory)?;
    let path = directory.join(format!("{id}.json"));
    let bytes = serde_json::to_vec(command)?;
    match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
    {
        Ok(file) => {
            ensure!(file.metadata()?.len() <= 16384, "invalid lifecycle receipt");
            let mut previous = Vec::new();
            file.take(16385).read_to_end(&mut previous)?;
            ensure!(previous == bytes, "lifecycle command ID payload conflict");
            return Ok(true);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if reserve {
        ensure!(
            std::fs::read_dir(&directory)?.take(65536).count() < 65536,
            "lifecycle receipt capacity exhausted"
        );
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        File::open(directory)?.sync_all()?;
    }
    Ok(false)
}
