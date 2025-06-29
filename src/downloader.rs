use std::{
    collections::{HashMap, HashSet},
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use compak::Archive;
use futures::{future::join_all, TryStreamExt};
use regex::Regex;
use reqwest::header::{HeaderMap, CONTENT_DISPOSITION, ETAG, LAST_MODIFIED};

use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    sync::Semaphore,
    task,
};
use url::Url;

use crate::{
    error::DownloadError,
    http_client::SHARED_CLIENT,
    oci::{OciClient, OciLayer, OciManifest, Reference},
    resume::ResumeSupport,
    utils::{
        build_absolute_path, default_prompt_confirm, extract_filename, extract_filename_from_url,
        is_elf, matches_pattern, FileMode,
    },
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
    pub extract_archive: bool,
    pub extract_dir: Option<String>,
    pub file_mode: FileMode,
    pub prompt: Option<Arc<dyn Fn(&str) -> Result<bool, DownloadError> + Send + Sync + 'static>>,
}

pub struct Downloader<'a> {
    client: &'a reqwest::Client,
}

#[derive(Clone)]
pub struct OciDownloadOptions {
    pub url: String,
    pub concurrency: Option<u64>,
    pub output_path: Option<String>,
    pub progress_callback: Option<Arc<dyn Fn(DownloadState) + Send + Sync + 'static>>,
    pub api: Option<String>,
    pub regexes: Vec<Regex>,
    pub globs: Vec<String>,
    pub match_keywords: Vec<String>,
    pub exclude_keywords: Vec<String>,
    pub exact_case: bool,
    pub file_mode: FileMode,
}

impl<'a> Default for Downloader<'a> {
    fn default() -> Self {
        Downloader {
            client: &SHARED_CLIENT,
        }
    }
}

impl Downloader<'_> {
    pub fn client(&self) -> &reqwest::Client {
        self.client
    }

    pub async fn download(&self, options: DownloadOptions) -> Result<String, DownloadError> {
        let url = Url::parse(&options.url).map_err(|err| DownloadError::InvalidUrl {
            url: options.url.clone(),
            source: err,
        })?;

        let hash_fallback = || {
            let mut hasher = blake3::Hasher::new();
            hasher.update(options.url.as_bytes());
            let result = hasher.finalize();
            result.to_hex().to_string()
        };

        let (provisional_path, final_dir) = if let Some(ref out) = options.output_path {
            if out.ends_with('/') {
                let dir = PathBuf::from(out);
                let base = extract_filename_from_url(&options.url).unwrap_or_else(hash_fallback);
                (dir.join(&base), Some(dir))
            } else {
                let p = PathBuf::from(out);
                if p.is_dir() {
                    let base =
                        extract_filename_from_url(&options.url).unwrap_or_else(hash_fallback);
                    (p.join(&base), Some(p))
                } else {
                    (p, None)
                }
            }
        } else {
            let base = extract_filename_from_url(&options.url).unwrap_or_else(hash_fallback);
            (PathBuf::from(&base), None)
        };

        if let Some(output_dir) = provisional_path.parent() {
            if !output_dir.exists() {
                fs::create_dir_all(output_dir).await?;
            }
        }

        let (part_path, meta_path) = ResumeSupport::get_part_paths(&provisional_path);
        let (mut etag, mut last_modified) = ResumeSupport::read_metadata(&meta_path).await?;

        let mut attempt = 0;
        let mut downloaded = if fs::try_exists(&part_path).await? {
            fs::metadata(&part_path).await?.len()
        } else {
            0
        };

        loop {
            let mut headers = HeaderMap::new();

            ResumeSupport::prepare_resume_headers(&mut headers, downloaded, &etag, &last_modified);

            let response = self
                .client
                .get(url.clone())
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
                    url: options.url,
                });
            }

            if options.output_path.as_deref() == Some("-") {
                let mut stdout = tokio::io::stdout();
                let mut stream = response.bytes_stream();

                while let Some(chunk) = stream
                    .try_next()
                    .await
                    .map_err(|_| DownloadError::ChunkError)?
                {
                    stdout.write_all(&chunk).await?;
                    stdout.flush().await?;
                }
                return Ok("-".to_string());
            }

            let header_name = response
                .headers()
                .get(CONTENT_DISPOSITION)
                .and_then(|header| header.to_str().ok())
                .and_then(extract_filename);

            let final_target = match &options.output_path {
                Some(_) => provisional_path,
                None => match header_name {
                    Some(ref name) => final_dir
                        .as_ref()
                        .map(|dir| dir.join(name))
                        .unwrap_or_else(|| PathBuf::from(name)),
                    None => provisional_path,
                },
            };

            if final_target.exists() && !part_path.exists() {
                match options.file_mode {
                    FileMode::SkipExisting => return Ok(final_target.to_string_lossy().into()),
                    FileMode::ForceOverwrite => {
                        fs::remove_file(&final_target).await.ok();
                    }
                    FileMode::PromptOverwrite => {
                        let target = final_target.to_string_lossy().to_string();
                        let proceed = if let Some(prompt) = &options.prompt {
                            prompt(&target)?
                        } else {
                            default_prompt_confirm(&target)?
                        };

                        if !proceed {
                            return Ok(target);
                        }
                    }
                }
            }

            let (should_truncate, total_size) =
                ResumeSupport::extract_range_info(&response, downloaded);

            if let Some(ref callback) = options.progress_callback {
                callback(DownloadState::Preparing(total_size));
            }

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
                file.write_all(&chunk).await?;
                downloaded += chunk.len() as u64;

                if let Some(ref callback) = options.progress_callback {
                    callback(DownloadState::Progress(downloaded));
                }
            }

            fs::rename(&part_path, &final_target).await?;
            fs::remove_file(&meta_path).await.ok();

            if is_elf(&final_target).await {
                fs::set_permissions(&final_target, Permissions::from_mode(0o755)).await?;
            }

            if options.extract_archive {
                let extract_dir = match &options.extract_dir {
                    Some(path) => PathBuf::from(path),
                    None => {
                        let path = build_absolute_path(&final_target)?;
                        path.parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| PathBuf::from("."))
                    }
                };
                let archive = Archive::new(&final_target)?;
                archive.extract_to(&extract_dir).await?;
            }

            if let Some(ref callback) = options.progress_callback {
                callback(DownloadState::Complete);
            }
            return Ok(final_target.to_string_lossy().into());
        }
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
        let oci_client = OciClient::new(
            &reference,
            options.api.clone(),
            self.options.file_mode.clone(),
        );

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
                    options.regexes.as_slice(),
                    options.globs.as_slice(),
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
