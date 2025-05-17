use std::path::Path;
use std::sync::Arc;
use std::{collections::HashMap, fs::Permissions, os::unix::fs::PermissionsExt};

use futures::TryStreamExt;
use reqwest::header::{self, HeaderMap, ETAG, LAST_MODIFIED};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
};

use crate::utils::FileMode;
use crate::{error::DownloadError, resume::ResumeSupport, utils::is_elf};

#[derive(Clone, Deserialize)]
pub struct OciLayer {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    pub size: u64,
    pub annotations: HashMap<String, String>,
}

#[derive(Clone, Deserialize)]
pub struct OciConfig {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Clone, Deserialize)]
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
    pub api: Option<String>,
    pub file_mode: FileMode,
    pub prompt: Option<Arc<dyn Fn(&str) -> Result<bool, DownloadError> + Send + Sync + 'static>>,
}

#[derive(Clone, Debug)]
pub struct OciDownloadProgress {
    pub url: String,
    pub downloaded_layers: Vec<String>,
    pub total_layers: Vec<String>,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
}

#[derive(Clone)]
pub struct Reference {
    pub package: String,
    pub tag: String,
}

#[derive(Deserialize, Serialize)]
pub struct LayerMeta {
    etag: Option<String>,
    last_modified: Option<String>,
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
    pub fn new(reference: &Reference, api: Option<String>, file_mode: FileMode) -> Self {
        let client = reqwest::Client::new();
        Self {
            client,
            reference: reference.clone(),
            api,
            file_mode,
            prompt: None,
        }
    }

    pub fn headers(&self) -> HeaderMap {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            header::ACCEPT,
            ("application/vnd.docker.distribution.manifest.v2+json, \
            application/vnd.docker.distribution.manifest.list.v2+json, \
            application/vnd.oci.image.manifest.v1+json, \
            application/vnd.oci.image.index.v1+json, \
            application/vnd.oci.artifact.manifest.v1+json")
                .parse()
                .unwrap(),
        );
        header_map.insert(header::AUTHORIZATION, "Bearer QQ==".parse().unwrap());
        header_map
    }

    pub async fn manifest(&self) -> Result<OciManifest, DownloadError> {
        let manifest_url = format!(
            "{}/{}/manifests/{}",
            self.api
                .clone()
                .unwrap_or("https://ghcr.io/v2".to_string())
                .trim_end_matches('/'),
            self.reference.package,
            self.reference.tag
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
        let output_path = output_path.as_ref();
        let (part_path, meta_path) = ResumeSupport::get_part_paths(output_path);
        let (mut etag, mut last_modified) = ResumeSupport::read_metadata(&meta_path).await?;

        let mut attempt = 0;
        let mut downloaded = if fs::try_exists(&part_path).await? {
            fs::metadata(&part_path).await?.len()
        } else {
            0
        };

        loop {
            let blob_url = format!(
                "{}/{}/blobs/{}",
                self.api
                    .clone()
                    .unwrap_or("https://ghcr.io/v2".to_string())
                    .trim_end_matches('/'),
                self.reference.package,
                layer.digest
            );

            let mut headers = self.headers();

            ResumeSupport::prepare_resume_headers(&mut headers, downloaded, &etag, &last_modified);

            let response = self
                .client
                .get(&blob_url)
                .headers(headers.clone())
                .send()
                .await
                .map_err(|err| DownloadError::NetworkError { source: err })?;

            let status = response.status();

            let remote_etag = response
                .headers()
                .get(ETAG)
                .and_then(|h| h.to_str().ok())
                .map(String::from);
            let remote_modified = response
                .headers()
                .get(LAST_MODIFIED)
                .and_then(|h| h.to_str().ok())
                .map(String::from);

            if ResumeSupport::should_restart_download(
                status,
                &etag,
                &last_modified,
                &remote_etag,
                &remote_modified,
            ) && attempt == 0
            {
                fs::remove_file(&part_path).await.ok();
                fs::remove_file(&meta_path).await.ok();
                etag = remote_etag.clone();
                last_modified = remote_modified.clone();
                downloaded = 0;
                attempt += 1;
                continue;
            }

            if !status.is_success() {
                return Err(DownloadError::ResourceError {
                    status,
                    url: blob_url,
                });
            }

            if output_path.exists() && !part_path.exists() {
                match self.file_mode {
                    FileMode::SkipExisting => return Ok(downloaded),
                    FileMode::ForceOverwrite => {
                        fs::remove_file(&output_path).await.ok();
                    }
                    FileMode::PromptOverwrite => {
                        // Note: prompt doesn't play nice with progress bar
                        // let it be same as ForceOverwrite for now
                        fs::remove_file(&output_path).await.ok();
                    }
                }
            }

            let (should_truncate, total_size) =
                ResumeSupport::extract_range_info(&response, downloaded);

            progress_callback(downloaded, total_size);

            let mut file = if should_truncate || downloaded == 0 {
                fs::remove_file(&part_path).await.ok();
                downloaded = 0;
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&part_path)
                    .await?
            } else {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&part_path)
                    .await?
            };

            ResumeSupport::write_metadata(&meta_path, remote_etag, remote_modified).await?;

            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream
                .try_next()
                .await
                .map_err(|_| DownloadError::ChunkError)?
            {
                let chunk_size = chunk.len() as u64;
                file.write_all(&chunk).await?;

                downloaded += chunk_size;
                progress_callback(chunk_size, 0);
            }

            fs::rename(&part_path, &output_path).await?;
            fs::remove_file(&meta_path).await.ok();

            if is_elf(&output_path).await {
                fs::set_permissions(&output_path, Permissions::from_mode(0o755)).await?;
            }

            return Ok(downloaded);
        }
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
