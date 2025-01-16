use std::{
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{Arc, Mutex},
};

use futures::{future::join_all, StreamExt};
use reqwest::header::USER_AGENT;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    task,
};
use url::Url;

use crate::{
    error::DownloadError,
    oci::{OciClient, Reference},
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

    pub async fn download_oci(&self, options: DownloadOptions) -> Result<(), DownloadError> {
        let url = options.url.clone();
        let reference: Reference = url.into();
        let oci_client = OciClient::new(reference);

        let manifest = oci_client.manifest().await.unwrap();

        let mut tasks = Vec::new();
        let total_bytes: u64 = manifest.layers.iter().map(|layer| layer.size).sum();

        if let Some(ref callback) = options.progress_callback {
            callback(DownloadState::Progress(DownloadProgress {
                bytes_downloaded: 0,
                total_bytes: Some(total_bytes),
                url: options.url.clone(),
                file_path: String::new(),
            }));
        }

        let downloaded_bytes = Arc::new(Mutex::new(0u64));
        let outdir = options.output_path;

        for layer in manifest.layers {
            let client_clone = oci_client.clone();
            let cb_clone = options.progress_callback.clone();
            let downloaded_bytes = downloaded_bytes.clone();
            let url = options.url.clone();
            let outdir = outdir.clone();

            let task = task::spawn(async move {
                let chunk_size = client_clone
                    .pull_layer(&layer, outdir, move |bytes| {
                        if let Some(ref callback) = cb_clone {
                            let mut current = downloaded_bytes.lock().unwrap();
                            *current = bytes;
                            callback(DownloadState::Progress(DownloadProgress {
                                bytes_downloaded: *current,
                                total_bytes: Some(total_bytes),
                                url: url.clone(),
                                file_path: String::new(),
                            }));
                        }
                    })
                    .await?;

                Ok::<u64, DownloadError>(chunk_size)
            });
            tasks.push(task);
        }

        let _ = join_all(tasks).await;

        if let Some(ref callback) = options.progress_callback {
            callback(DownloadState::Complete);
        }

        Ok(())
    }
}
