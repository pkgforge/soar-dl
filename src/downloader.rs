use std::{fs::Permissions, os::unix::fs::PermissionsExt, path::Path, sync::Arc};

use futures::StreamExt;
use reqwest::header::USER_AGENT;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
};
use url::Url;

use crate::{
    error::DownloadError,
    utils::{extract_filename, is_elf},
};

#[derive(Debug, Clone)]
pub enum DownloadState {
    Progress(DownloadProgress),
    Complete,
}

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub bytes_downloaded: u64,
    pub total_bytes: Option<u64>,
    pub url: String,
    pub file_path: String,
}

pub struct DownloadOptions {
    pub url: String,
    pub output_path: Option<String>,
    pub progress_callback: Option<Arc<dyn Fn(DownloadState) + Send + Sync + 'static>>,
}

#[derive(Default)]
pub struct Downloader {
    client: reqwest::Client,
}

impl Downloader {
    pub async fn download(&self, options: DownloadOptions) -> Result<String, DownloadError> {
        let url = Url::parse(&options.url).map_err(|err| DownloadError::InvalidUrl {
            url: options.url.clone(),
            source: err,
        })?;

        let response = self
            .client
            .get(url)
            .header(USER_AGENT, "pkgforge/soar")
            .send()
            .await
            .map_err(|err| DownloadError::NetworkError { source: err })?;

        if !response.status().is_success() {
            return Err(DownloadError::ResourceError {
                status: response.status(),
                url: options.url,
            });
        }

        let content_length = response.content_length();

        let filename = options
            .output_path
            .unwrap_or_else(|| extract_filename(&options.url));
        let filename = if filename.ends_with('/') {
            format!(
                "{}/{}",
                filename.trim_end_matches('/'),
                extract_filename(&options.url)
            )
        } else {
            filename
        };

        let output_path = Path::new(&filename);
        if let Some(output_dir) = output_path.parent() {
            if !output_dir.exists() {
                fs::create_dir_all(output_dir).await?;
            }
        }

        let temp_path = format!("{}.part", output_path.display());
        let mut stream = response.bytes_stream();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&temp_path)
            .await?;

        let mut downloaded_bytes = 0u64;
        let mut progress_callback = options.progress_callback;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            file.write_all(&chunk).await.unwrap();
            downloaded_bytes = downloaded_bytes.saturating_add(chunk.len() as u64);

            if let Some(ref mut callback) = progress_callback {
                callback(DownloadState::Progress(DownloadProgress {
                    bytes_downloaded: downloaded_bytes,
                    total_bytes: content_length,
                    url: options.url.clone(),
                    file_path: filename.clone(),
                }));
            }
        }

        fs::rename(&temp_path, &output_path).await?;

        if is_elf(output_path).await {
            fs::set_permissions(&output_path, Permissions::from_mode(0o755)).await?;
        }

        if let Some(ref cb) = progress_callback {
            cb(DownloadState::Complete);
        }

        Ok(filename)
    }
}
