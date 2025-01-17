use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use futures::StreamExt;
use reqwest::header::{self, HeaderMap};
use serde::Deserialize;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
};

use crate::error::DownloadError;

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
    reference: Reference,
}

#[derive(Clone)]
pub struct Reference {
    package: String,
    tag: String,
}

impl From<&str> for Reference {
    fn from(value: &str) -> Self {
        let paths = value.trim_start_matches("ghcr.io/");
        let (package, tag) = paths.split_once(':').unwrap_or((paths, "latest"));

        Self {
            package: package.to_string(),
            tag: tag.to_string(),
        }
    }
}

impl From<String> for Reference {
    fn from(value: String) -> Self {
        let paths = value.trim_start_matches("ghcr.io/");
        let (package, tag) = paths.split_once(':').unwrap_or((paths, "latest"));

        Self {
            package: package.to_string(),
            tag: tag.to_string(),
        }
    }
}

impl OciClient {
    pub fn new(reference: Reference) -> Self {
        let client = reqwest::Client::new();
        Self { client, reference }
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

    pub async fn pull_layer<F, P: AsRef<Path>>(
        &self,
        layer: &OciLayer,
        output_dir: Option<P>,
        progress_callback: F,
    ) -> Result<u64, DownloadError>
    where
        F: Fn(u64) + Send + 'static,
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

        let Some(filename) = layer.get_title() else {
            // skip if layer doesn't contain title
            return Ok(0);
        };

        let (temp_path, final_path) = if let Some(output_dir) = output_dir {
            let output_dir = output_dir.as_ref();
            fs::create_dir_all(output_dir).await?;
            let final_path = output_dir.join(format!("{filename}"));
            let temp_path = output_dir.join(format!("{filename}.part"));
            (temp_path, final_path)
        } else {
            let final_path = PathBuf::from(&filename);
            let temp_path = PathBuf::from(format!("{filename}.part"));
            (temp_path, final_path)
        };

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

            progress_callback(chunk_size);
            total_bytes_downloaded += chunk_size;
        }

        fs::rename(&temp_path, &final_path).await?;

        Ok(total_bytes_downloaded)
    }
}

impl OciLayer {
    pub fn get_title(&self) -> Option<String> {
        self.annotations
            .get("org.opencontainers.image.title")
            .cloned()
    }
}
