//! One-session native macOS admission path. Runtime execution remains in Voyage.
use super::{Phase, Registration, macos, unix_registry, unix_transport};
use anyhow::{Result, ensure};
use std::path::Path;
use voyage_protocol::process::{ProcessRegistration, RuntimeCommand, RuntimeResponse};

pub struct Session {
    process: macos::OwnedProcess,
    runtime: ProcessRegistration,
    directory: std::path::PathBuf,
    _lock: std::fs::File,
}
impl Session {
    /// Persist private runtime registration before spawning. Never replay an
    /// existing directory/receipt as a new launch after uncertain effects.
    pub fn start(
        binary: &Path,
        directory: &Path,
        native: Registration,
        runtime: ProcessRegistration,
    ) -> Result<Self> {
        ensure!(
            native.phase == Phase::SpawnUncertain
                && native.session_id == runtime.session_id
                && native.incarnation == runtime.incarnation
                && native.launch_command_id == runtime.command_id,
            "Native/runtime launch pins differ"
        );
        ensure!(
            !directory.exists(),
            "Existing native launch requires recovery, not another spawn"
        );
        unix_registry::directory(directory)?;
        let lock = unix_registry::lock(directory)?;
        unix_registry::publish(directory, &runtime)?;
        // Private durable native intent retains exact command and creation pins.
        let intent = directory.join("native-intent.json");
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(intent)?;
        file.write_all(&serde_json::to_vec(&native)?)?;
        file.sync_all()?;
        std::fs::File::open(directory)?.sync_all()?;
        let process = macos::spawn(
            binary,
            directory,
            &directory.join("native.log"),
            runtime.config_path.as_deref(),
            native,
        )?;
        Ok(Self {
            process,
            runtime,
            directory: directory.into(),
            _lock: lock,
        })
    }
    pub async fn request(&self, command: RuntimeCommand) -> Result<RuntimeResponse> {
        unix_transport::request(&self.directory, &self.runtime, command).await
    }
    pub fn process(&mut self) -> &mut macos::OwnedProcess {
        &mut self.process
    }
}
