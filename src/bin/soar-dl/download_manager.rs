use std::{io::Write, sync::Arc, thread, time::Duration};

use indicatif::HumanBytes;
use regex::Regex;
use reqwest::StatusCode;
use serde::Deserialize;
use soar_dl::{
    downloader::{DownloadOptions, DownloadState, Downloader, OciDownloadOptions, OciDownloader},
    error::{DownloadError, PlatformError},
    github::{Github, GithubAsset, GithubRelease},
    gitlab::{Gitlab, GitlabAsset, GitlabRelease},
    platform::{
        PlatformDownloadOptions, PlatformUrl, Release, ReleaseAsset, ReleaseHandler,
        ReleasePlatform,
    },
    utils::get_file_mode,
};

use crate::{cli::Args, error, info};

pub struct DownloadManager {
    args: Args,
    progress_callback: Arc<dyn Fn(DownloadState) + Send + Sync>,
}

impl DownloadManager {
    pub fn new(args: Args, progress_callback: Arc<dyn Fn(DownloadState) + Send + Sync>) -> Self {
        Self {
            args,
            progress_callback,
        }
    }

    pub async fn execute(&self) {
        let _ = self.handle_github_downloads().await;
        let _ = self.handle_oci_downloads().await;
        let _ = self.handle_gitlab_downloads().await;
        let _ = self.handle_direct_downloads().await;
    }

    fn create_regexes(&self) -> Vec<Regex> {
        self.args
            .regexes
            .clone()
            .map(|patterns| {
                patterns
                    .iter()
                    .map(|pattern| Regex::new(pattern))
                    .collect::<Result<Vec<Regex>, regex::Error>>()
            })
            .transpose()
            .unwrap()
            .unwrap_or_default()
    }

    fn create_platform_options(&self, tag: Option<String>) -> PlatformDownloadOptions {
        let regexes = self.create_regexes();
        PlatformDownloadOptions {
            output_path: self.args.output.clone(),
            progress_callback: Some(self.progress_callback.clone()),
            tag,
            regexes,
            globs: self.args.globs.clone().unwrap_or_default(),
            match_keywords: self.args.match_keywords.clone().unwrap_or_default(),
            exclude_keywords: self.args.exclude_keywords.clone().unwrap_or_default(),
            exact_case: false,
            extract_archive: self.args.extract,
            extract_dir: self.args.extract_dir.clone(),
            file_mode: get_file_mode(self.args.skip_existing, self.args.force_overwrite),
            prompt: Arc::new(prompt_confirm),
        }
    }

    async fn handle_platform_download<P: ReleasePlatform, R, A>(
        &self,
        handler: &ReleaseHandler<'_, P>,
        project: &str,
    ) -> Result<(), PlatformError>
    where
        R: Release<A> + for<'de> Deserialize<'de>,
        A: ReleaseAsset + Clone,
    {
        let (project, tag) = match project.trim().split_once('@') {
            Some((proj, tag)) if !tag.trim().is_empty() => (proj, Some(tag.trim())),
            _ => (project.trim_end_matches('@'), None),
        };

        let options = self.create_platform_options(tag.map(String::from));
        let releases = handler.fetch_releases::<R>(project, tag).await?;
        let assets = handler.filter_releases(&releases, &options).await?;

        let selected_asset = self.select_asset(&assets)?;

        info!("Downloading asset from {}", selected_asset.download_url());
        handler.download(&selected_asset, options.clone()).await?;
        Ok(())
    }

    async fn handle_github_downloads(&self) -> Result<(), PlatformError> {
        if self.args.github.is_empty() {
            return Ok(());
        }

        let handler = ReleaseHandler::<Github>::new();
        for project in &self.args.github {
            info!("Fetching releases from GitHub: {}", project);
            if let Err(e) = self
                .handle_platform_download::<Github, GithubRelease, GithubAsset>(&handler, project)
                .await
            {
                error!("{}", e);
            }
        }
        Ok(())
    }

    async fn handle_gitlab_downloads(&self) -> Result<(), PlatformError> {
        if self.args.gitlab.is_empty() {
            return Ok(());
        }

        let handler = ReleaseHandler::<Gitlab>::new();
        for project in &self.args.gitlab {
            info!("Fetching releases from GitLab: {}", project);
            if let Err(e) = self
                .handle_platform_download::<Gitlab, GitlabRelease, GitlabAsset>(&handler, project)
                .await
            {
                error!("{}", e);
            }
        }
        Ok(())
    }

    async fn handle_oci_download(&self, reference: &str) -> Result<(), PlatformError> {
        let regexes = self.create_regexes();
        let options = OciDownloadOptions {
            url: reference.to_string(),
            concurrency: self.args.concurrency.clone(),
            output_path: self.args.output.clone(),
            progress_callback: Some(self.progress_callback.clone()),
            api: self.args.ghcr_api.clone(),
            regexes,
            globs: self.args.globs.clone().unwrap_or_default(),
            match_keywords: self.args.match_keywords.clone().unwrap_or_default(),
            exclude_keywords: self.args.exclude_keywords.clone().unwrap_or_default(),
            exact_case: self.args.exact_case,
            file_mode: get_file_mode(self.args.skip_existing, self.args.force_overwrite),
        };
        let mut downloader = OciDownloader::new(options);
        let mut retries = 0;
        loop {
            if retries > 5 {
                error!("Max retries exhausted. Aborting.");
                break;
            }
            match downloader.download_oci().await {
                Ok(_) => break,
                Err(
                    DownloadError::ResourceError {
                        status: StatusCode::TOO_MANY_REQUESTS,
                        ..
                    }
                    | DownloadError::ChunkError,
                ) => thread::sleep(Duration::from_secs(5)),
                Err(err) => {
                    error!("{}", err);
                    break;
                }
            };
            retries += 1;
        }

        Ok(())
    }

    async fn handle_oci_downloads(&self) -> Result<(), PlatformError> {
        if self.args.ghcr.is_empty() {
            return Ok(());
        }

        for reference in &self.args.ghcr {
            info!("Downloading using OCI reference: {}", reference);

            self.handle_oci_download(reference).await?;
        }
        Ok(())
    }

    async fn handle_direct_downloads(&self) -> Result<(), DownloadError> {
        let downloader = Downloader::default();
        for link in &self.args.links {
            match PlatformUrl::parse(link) {
                Ok(PlatformUrl::DirectUrl(url)) => {
                    info!("Downloading using direct link: {}", url);

                    let options = DownloadOptions {
                        url: link.clone(),
                        output_path: self.args.output.clone(),
                        progress_callback: Some(self.progress_callback.clone()),
                        extract_archive: self.args.extract,
                        extract_dir: self.args.extract_dir.clone(),
                        file_mode: get_file_mode(
                            self.args.skip_existing,
                            self.args.force_overwrite,
                        ),
                        prompt: Arc::new(prompt_confirm),
                    };
                    let _ = downloader
                        .download(options)
                        .await
                        .map_err(|e| error!("{}", e));
                }
                Ok(PlatformUrl::Github(project)) => {
                    info!("Detected GitHub URL, processing as GitHub release");
                    let handler = ReleaseHandler::<Github>::new();
                    if let Err(e) = self
                        .handle_platform_download::<Github, GithubRelease, GithubAsset>(
                            &handler, &project,
                        )
                        .await
                    {
                        error!("{}", e);
                    }
                }
                Ok(PlatformUrl::Gitlab(project)) => {
                    info!("Detected GitLab URL, processing as GitLab release");
                    let handler = ReleaseHandler::<Gitlab>::new();
                    if let Err(e) = self
                        .handle_platform_download::<Gitlab, GitlabRelease, GitlabAsset>(
                            &handler, &project,
                        )
                        .await
                    {
                        error!("{}", e);
                    }
                }
                Ok(PlatformUrl::Oci(url)) => {
                    info!("Downloading using OCI reference: {}", url);
                    if let Err(e) = self.handle_oci_download(&url).await {
                        error!("{}", e);
                    };
                }
                Err(err) => error!("Error parsing URL '{}' : {}", link, err),
            };
        }
        Ok(())
    }

    fn select_asset<A>(&self, assets: &[A]) -> Result<A, DownloadError>
    where
        A: Clone,
        A: ReleaseAsset,
    {
        if assets.len() == 1 || self.args.yes {
            return Ok(assets[0].clone());
        }

        info!("\nAvailable assets:");
        for (i, asset) in assets.iter().enumerate() {
            let size = asset
                .size()
                .map(|s| format!(" ({})", HumanBytes(s)))
                .unwrap_or_default();
            info!("{}. {}{}", i + 1, asset.name(), size);
        }

        loop {
            print!("\nSelect an asset (1-{}): ", assets.len());
            std::io::Write::flush(&mut std::io::stdout())?;

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;

            match input.trim().parse::<usize>() {
                Ok(n) if n > 0 && n <= assets.len() => return Ok(assets[n - 1].clone()),
                _ => error!("Invalid selection, please try again."),
            }
        }
    }
}

fn prompt_confirm(file_name: &str) -> Result<bool, DownloadError> {
    print!("Overwrite {}? [y/N] ", file_name);
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}
