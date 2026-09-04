use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Enrollment {
    pub helm_id: Uuid,
    pub vessel_url: String,
    pub worker_token: String,
    pub name: String,
}

pub struct EnrollmentStore {
    path: PathBuf,
}

impl EnrollmentStore {
    pub fn default_path() -> Result<PathBuf> {
        Ok(dirs::config_dir()
            .context("could not determine config directory")?
            .join("helm/voyage.json"))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub async fn load(&self) -> Result<Option<Enrollment>> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).context("invalid Voyage enrollment")?,
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context("could not read Voyage enrollment"),
        }
    }

    pub async fn save(&self, enrollment: &Enrollment) -> Result<()> {
        let parent = self.path.parent().context("invalid enrollment path")?;
        tokio::fs::create_dir_all(parent).await?;
        let temporary = self.path.with_extension(format!("tmp-{}", Uuid::new_v4()));
        tokio::fs::write(&temporary, serde_json::to_vec_pretty(enrollment)?).await?;
        restrict(&temporary)?;
        tokio::fs::rename(&temporary, &self.path).await?;
        restrict(&self.path)?;
        Ok(())
    }

    pub async fn remove(&self) -> Result<()> {
        match tokio::fs::remove_file(&self.path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(unix)]
fn restrict(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}
#[cfg(not(unix))]
fn restrict(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn normalize_vessel_url(value: &str) -> Result<String> {
    let value = value.trim_end_matches('/');
    if !(value.starts_with("https://")
        || value.starts_with("http://127.0.0.1:")
        || value.starts_with("http://localhost:"))
    {
        bail!("Vessel must use HTTPS (HTTP is accepted only for loopback development)");
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn enrollment_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::new(dir.path().join("enrollment.json"));
        let value = Enrollment {
            helm_id: Uuid::new_v4(),
            vessel_url: "http://127.0.0.1:1".into(),
            worker_token: "secret".into(),
            name: "test".into(),
        };
        store.save(&value).await.unwrap();
        assert_eq!(store.load().await.unwrap().unwrap().helm_id, value.helm_id);
    }
}
