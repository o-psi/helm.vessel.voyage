use anyhow::{Result, ensure};
use serde::Deserialize;
use std::{collections::BTreeSet, path::Path, time::Duration};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    format: u32,
    url: String,
    sha256: String,
    /// Optional explicit PEM root for a private index. Never read from a package.
    ca_certificate: Option<std::path::PathBuf>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    format: u32,
    packages: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    url: String,
    sha256: String,
}
fn url(raw: &str) -> Result<reqwest::Url> {
    ensure!(raw.len() <= 2048, "index URL exceeds limit");
    let url = reqwest::Url::parse(raw).map_err(|_| anyhow::anyhow!("invalid index URL"))?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "index URLs require HTTPS without credentials, queries or fragments"
    );
    Ok(url)
}
async fn download(client: &reqwest::Client, url: reqwest::Url, sha: &str) -> Result<Vec<u8>> {
    ensure!(super::hash(sha), "invalid expected download digest");
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("package HTTPS request failed"))?;
    ensure!(
        response.status().is_success(),
        "package HTTPS status refused"
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|len| len <= super::MAX_ARCHIVE as u64),
        "package download exceeds limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("package HTTPS body failed"))?
    {
        ensure!(
            bytes.len() + chunk.len() <= super::MAX_ARCHIVE,
            "package download exceeds limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    ensure!(super::digest(&bytes) == sha, "download integrity mismatch");
    Ok(bytes)
}
pub(super) async fn acquire(config: &Path, id: &str) -> Result<Vec<u8>> {
    let configuration: Configuration = serde_json::from_slice(&super::store::local(config)?)
        .map_err(|_| anyhow::anyhow!("invalid index configuration"))?;
    ensure!(
        configuration.format == 1 && super::identifier(id),
        "invalid index format or identity"
    );
    let origin = url(&configuration.url)?;
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(5));
    if let Some(path) = &configuration.ca_certificate {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            config.parent().unwrap_or(Path::new(".")).join(path)
        };
        let bytes = super::store::local(&path)?;
        builder = builder.add_root_certificate(
            reqwest::Certificate::from_pem(&bytes)
                .map_err(|_| anyhow::anyhow!("invalid index CA certificate"))?,
        );
    }
    let client = builder.build()?;
    let raw = download(&client, origin.clone(), &configuration.sha256).await?;
    let index: Index =
        serde_json::from_slice(&raw).map_err(|_| anyhow::anyhow!("invalid package index"))?;
    ensure!(
        index.format == 1 && index.packages.len() <= 128,
        "unsupported or oversized index"
    );
    let mut ids = BTreeSet::new();
    for entry in &index.packages {
        ensure!(
            super::identifier(&entry.id) && ids.insert(&entry.id) && super::hash(&entry.sha256),
            "invalid or duplicate index identity/hash"
        );
        ensure!(
            url(&entry.url)?.origin() == origin.origin(),
            "package URL must share index HTTPS origin"
        );
    }
    let entry = index
        .packages
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow::anyhow!("package absent from index"))?;
    let bytes = download(&client, url(&entry.url)?, &entry.sha256).await?;
    ensure!(
        super::Archive::parse(&bytes)?.manifest.id == id,
        "downloaded identity mismatch"
    );
    Ok(bytes)
}
