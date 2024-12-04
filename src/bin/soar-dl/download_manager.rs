use std::sync::Arc;

use indicatif::HumanBytes;
use regex::Regex;
use serde::Deserialize;
use soar_dl::{
    common::{PlatformDownloadOptions, Release, ReleaseAsset, ReleaseHandler, ReleasePlatform},
    downloader::{DownloadOptions, DownloadState, Downloader},
    error::{DownloadError, PlatformError},
    github::{Github, GithubAsset, GithubRelease},
    gitlab::{Gitlab, GitlabAsset, GitlabRelease},
};

use crate::cli::Args;

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
        let options = self.create_platform_options();

        let _ = self.handle_github_downloads(&options).await;
        let _ = self.handle_gitlab_downloads(&options).await;
        let _ = self.handle_direct_downloads().await;
    }

    fn create_platform_options(&self) -> PlatformDownloadOptions {
        let asset_regexes = self
            .args
            .regex_patterns
            .clone()
            .map(|patterns| {
                patterns
                    .iter()
                    .map(|pattern| Regex::new(pattern))
                    .collect::<Result<Vec<Regex>, regex::Error>>()
            })
            .transpose()
            .unwrap()
            .unwrap_or_default();

        PlatformDownloadOptions {
            output_path: self.args.output.clone(),
            progress_callback: Some(self.progress_callback.clone()),
            tag: None,
            regex_patterns: asset_regexes,
            match_keywords: self.args.match_keywords.clone().unwrap_or_default(),
            exclude_keywords: self.args.exclude_keywords.clone().unwrap_or_default(),
            exact_case: false,
        }
    }

    async fn handle_platform_download<P: ReleasePlatform, R, A>(
        &self,
        handler: &ReleaseHandler<P>,
        project: &str,
        options: &PlatformDownloadOptions,
    ) -> Result<(), PlatformError>
    where
        R: Release<A> + for<'de> Deserialize<'de>,
        A: ReleaseAsset + Clone,
    {
        let releases = handler.fetch_releases::<R>(project).await?;
        let assets = handler.filter_releases(&releases, options).await?;

        let selected_asset = self.select_asset(&assets)?;
        handler.download(&selected_asset, options.clone()).await?;
        Ok(())
    }

    async fn handle_github_downloads(
        &self,
        options: &PlatformDownloadOptions,
    ) -> Result<(), PlatformError> {
        if self.args.github.is_empty() {
            return Ok(());
        }

        let handler = ReleaseHandler::<Github>::new();
        for project in &self.args.github {
            println!("Fetching releases from GitHub: {}", project);
            if let Err(e) = self
                .handle_platform_download::<Github, GithubRelease, GithubAsset>(
                    &handler, project, options,
                )
                .await
            {
                eprintln!("{}", e);
            }
        }
        Ok(())
    }

    async fn handle_gitlab_downloads(
        &self,
        options: &PlatformDownloadOptions,
    ) -> Result<(), PlatformError> {
        if self.args.gitlab.is_empty() {
            return Ok(());
        }

        let handler = ReleaseHandler::<Gitlab>::new();
        for project in &self.args.gitlab {
            println!("Fetching releases from GitLab: {}", project);
            if let Err(e) = self
                .handle_platform_download::<Gitlab, GitlabRelease, GitlabAsset>(
                    &handler, project, options,
                )
                .await
            {
                eprintln!("{}", e);
            }
        }
        Ok(())
    }

    async fn handle_direct_downloads(&self) -> Result<(), DownloadError> {
        let downloader = Downloader::default();
        for link in &self.args.links {
            let options = DownloadOptions {
                url: link.clone(),
                output_path: self.args.output.clone(),
                progress_callback: Some(self.progress_callback.clone()),
            };

            println!("Downloading using direct link: {}", link);
            let _ = downloader
                .download(options)
                .await
                .map_err(|e| eprintln!("{}", e));
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

        println!("\nAvailable assets:");
        for (i, asset) in assets.iter().enumerate() {
            let size = asset
                .size()
                .map(|s| format!(" ({})", HumanBytes(s)))
                .unwrap_or_default();
            println!("{}. {}{}", i + 1, asset.name(), size);
        }

        loop {
            print!("\nSelect an asset (1-{}): ", assets.len());
            std::io::Write::flush(&mut std::io::stdout())?;

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;

            match input.trim().parse::<usize>() {
                Ok(n) if n > 0 && n <= assets.len() => return Ok(assets[n - 1].clone()),
                _ => println!("Invalid selection, please try again."),
            }
        }
    }
}
