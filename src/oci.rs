use std::{
    collections::HashMap,
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use futures::StreamExt;
use reqwest::header::{self, HeaderMap};
use serde::Deserialize;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
};

use crate::{error::DownloadError, utils::is_elf};

#[derive(Deserialize)]
pub struct OciLayer {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    pub size: u64,
    pub annotations: HashMap<String, String>,
}

#[derive(Deserialize)]
pub struct OciConfig {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Deserialize)]
pub struct OciManifest {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub config: OciConfig,
    pub layers: Vec<OciLayer>,
}

#[derive(Clone)]
pub struct OciClient {
    client: reqwest::Client,
    pub reference: Reference,
}

#[derive(Clone)]
pub struct Reference {
    pub package: String,
    pub tag: String,
}

impl From<&str> for Reference {
    fn from(value: &str) -> Self {
        let paths = value.trim_start_matches("ghcr.io/");

        // <package>@sha256:<digest>
        if let Some((package, digest)) = paths.split_once("@") {
            return Self {
                package: package.to_string(),
                tag: digest.to_string(),
            };
        }

        // <package>:<tag>
        if let Some((package, tag)) = paths.split_once(':') {
            return Self {
                package: package.to_string(),
                tag: tag.to_string(),
            };
        }

        Self {
            package: paths.to_string(),
            tag: "latest".to_string(),
        }
    }
}

impl From<String> for Reference {
    fn from(value: String) -> Self {
        value.as_str().into()
    }
}

impl OciClient {
    pub fn new(reference: &Reference) -> Self {
        let client = reqwest::Client::new();
        Self {
            client,
            reference: reference.clone(),
        }
    }

    pub fn headers(&self) -> HeaderMap {
        let mut header_map = HeaderMap::new();
        header_map.insert(header::ACCEPT, "application/vnd.docker.distribution.manifest.v2+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.oci.image.index.v1+json, application/vnd.oci.artifact.manifest.v1+json".parse().unwrap());
        header_map.insert(header::AUTHORIZATION, "Bearer QQ==".parse().unwrap());
        header_map
    }

    pub async fn manifest(&self) -> Result<OciManifest, DownloadError> {
        let manifest_url = format!(
            "https://ghcr.io/v2/{}/manifests/{}",
            self.reference.package, self.reference.tag
        );
        let resp = self
            .client
            .get(&manifest_url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|err| DownloadError::NetworkError { source: err })?;

        if !resp.status().is_success() {
            return Err(DownloadError::ResourceError {
                status: resp.status(),
                url: manifest_url,
            });
        }

        let manifest: OciManifest = resp
            .json()
            .await
            .map_err(|_| DownloadError::InvalidResponse)?;
        Ok(manifest)
    }

    pub async fn pull_layer<F, P>(
        &self,
        layer: &OciLayer,
        output_path: P,
        progress_callback: F,
    ) -> Result<u64, DownloadError>
    where
        P: AsRef<Path>,
        F: Fn(u64, u64) + Send + 'static,
    {
        let blob_url = format!(
            "https://ghcr.io/v2/{}/blobs/{}",
            self.reference.package, layer.digest
        );
        let resp = self
            .client
            .get(&blob_url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|err| DownloadError::NetworkError { source: err })?;

        if !resp.status().is_success() {
            return Err(DownloadError::ResourceError {
                status: resp.status(),
                url: blob_url,
            });
        }

        let content_length = resp.content_length().unwrap_or(0);
        progress_callback(0, content_length);

        let output_path = output_path.as_ref();
        let temp_path = PathBuf::from(&format!("{}.part", output_path.display()));

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&temp_path)
            .await?;

        let mut stream = resp.bytes_stream();
        let mut total_bytes_downloaded = 0;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            let chunk_size = chunk.len() as u64;
            file.write_all(&chunk).await.unwrap();

            progress_callback(chunk_size, 0);
            total_bytes_downloaded += chunk_size;
        }

        fs::rename(&temp_path, &output_path).await?;

        if is_elf(&output_path).await {
            fs::set_permissions(&output_path, Permissions::from_mode(0o755)).await?;
        }

        Ok(total_bytes_downloaded)
    }
}

impl OciLayer {
    pub fn get_title(&self) -> Option<String> {
        self.annotations
            .get("org.opencontainers.image.title")
            .cloned()
    }

    pub fn set_title(&mut self, title: &str) {
        self.annotations.insert(
            "org.opencontainers.image.title".to_string(),
            title.to_string(),
        );
    }
}
