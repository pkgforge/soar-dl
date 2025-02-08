use std::{
    collections::{HashMap, HashSet},
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use futures::{future::join_all, StreamExt};
use regex::Regex;
use reqwest::header::USER_AGENT;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    sync::Semaphore,
    task,
};
use url::Url;

use crate::{
    error::DownloadError,
    oci::{OciClient, OciLayer, OciManifest, Reference},
    utils::{extract_filename, is_elf, matches_pattern},
};

#[derive(Debug, Clone)]
pub enum DownloadState {
    Preparing(u64),
    Progress(u64),
    Complete,
    Error,
    Aborted,
    Recovered,
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

#[derive(Clone)]
pub struct OciDownloadOptions {
    pub url: String,
    pub concurrency: Option<u64>,
    pub output_path: Option<String>,
    pub progress_callback: Option<Arc<dyn Fn(DownloadState) + Send + Sync + 'static>>,
    pub api: Option<String>,
    pub regex_patterns: Vec<Regex>,
    pub match_keywords: Vec<String>,
    pub exclude_keywords: Vec<String>,
    pub exact_case: bool,
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

        let content_length = response.content_length().unwrap_or(0);

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
        let progress_callback = options.progress_callback;

        if let Some(ref callback) = progress_callback {
            callback(DownloadState::Preparing(content_length));
        }

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            file.write_all(&chunk).await.unwrap();
            downloaded_bytes = downloaded_bytes.saturating_add(chunk.len() as u64);

            if let Some(ref callback) = progress_callback {
                callback(DownloadState::Progress(downloaded_bytes));
            }
        }

        fs::rename(&temp_path, &output_path).await?;

        if is_elf(output_path).await {
            fs::set_permissions(&output_path, Permissions::from_mode(0o755)).await?;
        }

        if let Some(ref callback) = progress_callback {
            callback(DownloadState::Complete);
        }

        Ok(filename)
    }
}

pub struct OciDownloader {
    manifest: Option<OciManifest>,
    options: OciDownloadOptions,
    completed_layers: Arc<Mutex<HashSet<String>>>,
}

impl OciDownloader {
    pub fn new(options: OciDownloadOptions) -> Self {
        Self {
            manifest: None,
            options,
            completed_layers: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn download_blob(&self, client: OciClient) -> Result<(), DownloadError> {
        let options = &self.options;
        let reference = client.reference.clone();
        let digest = reference.tag;
        let downloaded_bytes = Arc::new(Mutex::new(0u64));
        let output_path = options.output_path.clone();
        let ref_name = reference
            .package
            .rsplit_once('/')
            .map_or(digest.clone(), |(_, name)| name.to_string());
        let file_path = output_path.unwrap_or_else(|| ref_name.clone());
        let file_path = if file_path.ends_with('/') {
            fs::create_dir_all(&file_path).await?;
            format!("{}/{}", file_path.trim_end_matches('/'), ref_name)
        } else {
            file_path
        };

        let fake_layer = OciLayer {
            media_type: String::from("application/octet-stream"),
            digest: digest.clone(),
            size: 0,
            annotations: HashMap::new(),
        };

        let cb_clone = options.progress_callback.clone();
        client
            .pull_layer(&fake_layer, &file_path, move |bytes, total_bytes| {
                if let Some(ref callback) = cb_clone {
                    if total_bytes > 0 {
                        callback(DownloadState::Preparing(total_bytes));
                    }
                    let mut current = downloaded_bytes.lock().unwrap();
                    *current += bytes;
                    callback(DownloadState::Progress(*current));
                }
            })
            .await?;

        if let Some(ref callback) = options.progress_callback {
            callback(DownloadState::Complete);
        }

        Ok(())
    }

    pub async fn download_oci(&mut self) -> Result<(), DownloadError> {
        let options = &self.options;
        let url = options.url.clone();
        let reference: Reference = url.into();
        let oci_client = OciClient::new(&reference, options.api.clone());

        if reference.tag.starts_with("sha256:") {
            return self.download_blob(oci_client).await;
        }
        let manifest = match self.manifest {
            Some(ref manifest) => manifest,
            None => &oci_client.manifest().await?,
        };

        let mut tasks = Vec::new();

        let layers = manifest
            .layers
            .iter()
            .filter(|layer| {
                let Some(title) = layer.get_title() else {
                    return false;
                };

                matches_pattern(
                    &title,
                    options.regex_patterns.as_slice(),
                    options.match_keywords.as_slice(),
                    options.exclude_keywords.as_slice(),
                    options.exact_case,
                )
            })
            .cloned()
            .collect::<Vec<_>>();

        if layers.is_empty() {
            return Err(DownloadError::LayersNotFound);
        }

        let total_bytes: u64 = layers.iter().map(|layer| layer.size).sum();

        if let Some(ref callback) = options.progress_callback {
            callback(DownloadState::Preparing(total_bytes));
        }

        let semaphore = Arc::new(Semaphore::new(options.concurrency.unwrap_or(1) as usize));
        let downloaded_bytes = Arc::new(Mutex::new(0u64));
        let outdir = options.output_path.clone();
        let base_path = if let Some(dir) = outdir {
            fs::create_dir_all(&dir).await?;
            PathBuf::from(dir)
        } else {
            PathBuf::new()
        };

        for layer in layers {
            if self
                .completed_layers
                .lock()
                .unwrap()
                .contains(&layer.digest)
            {
                continue;
            }
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let client_clone = oci_client.clone();
            let cb_clone = options.progress_callback.clone();
            let downloaded_bytes = downloaded_bytes.clone();
            let completed_layers = self.completed_layers.clone();
            let Some(filename) = layer.get_title() else {
                continue;
            };

            let file_path = base_path.join(filename);

            let task = task::spawn(async move {
                client_clone
                    .pull_layer(&layer, &file_path, move |bytes, _| {
                        if let Some(ref callback) = cb_clone {
                            let mut current = downloaded_bytes.lock().unwrap();
                            *current += bytes;
                            callback(DownloadState::Progress(*current));
                        }
                    })
                    .await?;
                completed_layers.lock().unwrap().insert(layer.digest);

                Ok::<(), DownloadError>(())
            });
            drop(permit);
            tasks.push(task);
        }

        let _ = join_all(tasks).await;

        if let Some(ref callback) = options.progress_callback {
            callback(DownloadState::Complete);
        }

        Ok(())
    }
}
