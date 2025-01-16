use std::{
    env,
    sync::{Arc, LazyLock},
};

use regex::Regex;
use reqwest::header::{HeaderMap, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;

use crate::{
    downloader::{DownloadOptions, DownloadState, Downloader},
    error::{DownloadError, PlatformError},
    utils::should_fallback,
};

pub enum ApiType {
    PkgForge,
    Primary,
}

#[derive(Debug)]
pub enum PlatformUrl {
    Github(String),
    Gitlab(String),
    Oci(String),
    DirectUrl(String),
}

static GITHUB_RELEASE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?i)(?:https?://)?(?:github(?:\.com)?[:/])([^/@]+/[^/@]+)(?:@([^/\s]*)?)?$")
        .unwrap()
});
static GITLAB_RELEASE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?i)(?:https?://)?(?:gitlab(?:\.com)?[:/])([^/@]+/[^/@]+)(?:@([^/\s]*)?)?$")
        .unwrap()
});

impl PlatformUrl {
    pub fn parse(url: impl Into<String>) -> Result<Self, PlatformError> {
        let url = url.into();
        if url.starts_with("ghcr.io") {
            return Ok(PlatformUrl::Oci(url));
        }
        if GITHUB_RELEASE_RE.is_match(&url) {
            if let Some(caps) = GITHUB_RELEASE_RE.captures(&url) {
                let project = caps.get(1).unwrap().as_str();
                let tag = caps
                    .get(2)
                    .map(|tag| tag.as_str().trim())
                    .filter(|&tag| !tag.is_empty());
                if let Some(tag) = tag {
                    return Ok(PlatformUrl::Github(format!("{}@{}", project, tag)));
                } else {
                    return Ok(PlatformUrl::Github(project.to_string()));
                }
            }
            return Err(PlatformError::InvalidInput(url));
        }
        if GITLAB_RELEASE_RE.is_match(&url) {
            if let Some(caps) = GITLAB_RELEASE_RE.captures(&url) {
                let project = caps.get(1).unwrap().as_str();
                let tag = caps
                    .get(2)
                    .map(|tag| tag.as_str().trim())
                    .filter(|&tag| !tag.is_empty());
                if let Some(tag) = tag {
                    return Ok(PlatformUrl::Gitlab(format!("{}@{}", project, tag)));
                } else {
                    return Ok(PlatformUrl::Gitlab(project.to_string()));
                }
            }
            return Err(PlatformError::InvalidInput(url));
        }
        Ok(PlatformUrl::DirectUrl(url))
    }
}

pub trait DownloadableAsset {
    fn name(&self) -> &str;
    fn size(&self) -> u64;
    fn download_url(&self) -> &str;
}

pub trait ReleasePlatform {
    const API_BASE_PRIMARY: &'static str;
    const API_BASE_PKGFORGE: &'static str;
    const TOKEN_ENV_VAR: &'static str;

    fn format_project_path(project: &str) -> Result<(String, String), PlatformError>;
    fn format_api_path(project: &str) -> Result<String, PlatformError>;
}

pub trait ReleaseAsset {
    fn name(&self) -> &str;
    fn size(&self) -> Option<u64>;
    fn download_url(&self) -> &str;
}

pub trait Release<A: ReleaseAsset> {
    fn name(&self) -> &str;
    fn tag_name(&self) -> &str;
    fn is_prerelease(&self) -> bool;
    fn published_at(&self) -> &str;
    fn assets(&self) -> Vec<A>;
}

#[derive(Clone)]
pub struct PlatformDownloadOptions {
    pub output_path: Option<String>,
    pub progress_callback: Option<Arc<dyn Fn(DownloadState) + Send + Sync + 'static>>,
    pub tag: Option<String>,
    pub regex_patterns: Vec<Regex>,
    pub match_keywords: Vec<String>,
    pub exclude_keywords: Vec<String>,
    pub exact_case: bool,
}

#[derive(Default)]
pub struct ReleaseHandler<P: ReleasePlatform> {
    downloader: Downloader,
    client: reqwest::Client,
    _platform: std::marker::PhantomData<P>,
}

impl<P: ReleasePlatform> ReleaseHandler<P> {
    pub fn new() -> Self {
        Self {
            downloader: Downloader::default(),
            client: reqwest::Client::new(),
            _platform: std::marker::PhantomData,
        }
    }

    async fn call_api(
        &self,
        api_type: &ApiType,
        project: &str,
    ) -> Result<reqwest::Response, PlatformError> {
        let base_url = match api_type {
            ApiType::PkgForge => P::API_BASE_PKGFORGE,
            ApiType::Primary => P::API_BASE_PRIMARY,
        };

        let api_path = P::format_api_path(project)?;
        let url = format!("{}{}", base_url, api_path);

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, "pkgforge/soar".parse().unwrap());

        if matches!(api_type, ApiType::Primary) {
            if let Ok(token) = env::var(P::TOKEN_ENV_VAR) {
                headers.insert(AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());
            }
        }

        Ok(self
            .client
            .get(&url)
            .headers(headers)
            .send()
            .await
            .map_err(|err| DownloadError::NetworkError { source: err })?)
    }

    pub async fn fetch_releases<R>(&self, project: &str) -> Result<Vec<R>, PlatformError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = match self.call_api(&ApiType::PkgForge, project).await {
            Ok(resp) => {
                let status = resp.status();
                if should_fallback(status) {
                    self.call_api(&ApiType::Primary, project).await?
                } else {
                    resp
                }
            }
            Err(err) => return Err(err),
        };

        if !response.status().is_success() {
            return Err(DownloadError::ResourceError {
                url: response.url().to_string(),
                status: response.status(),
            }
            .into());
        }

        response
            .json()
            .await
            .map_err(|_| PlatformError::InvalidResponse)
    }

    pub async fn filter_releases<R, A>(
        &self,
        releases: &[R],
        options: &PlatformDownloadOptions,
    ) -> Result<Vec<A>, PlatformError>
    where
        R: Release<A>,
        A: ReleaseAsset + Clone,
    {
        let release = if let Some(ref tag_name) = options.tag {
            releases
                .iter()
                .find(|release| release.tag_name() == tag_name)
        } else {
            releases
                .iter()
                .find(|release| !release.is_prerelease())
                .map_or_else(|| releases.first(), Some)
        };

        let Some(release) = release else {
            return Err(PlatformError::NoRelease {
                tag: options.tag.clone(),
            });
        };

        let assets: Vec<A> = release
            .assets()
            .into_iter()
            .filter(|asset| {
                let name = asset.name();
                options
                    .regex_patterns
                    .iter()
                    .all(|regex| regex.is_match(name))
                    && options.match_keywords.iter().all(|keyword| {
                        keyword
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .all(|part| {
                                let (asset_name, part) = if options.exact_case {
                                    (name.to_string(), part.to_string())
                                } else {
                                    (name.to_lowercase(), part.to_lowercase())
                                };
                                asset_name.contains(&part)
                            })
                    })
                    && options.exclude_keywords.iter().all(|keyword| {
                        keyword
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .all(|part| {
                                let (asset_name, part) = if options.exact_case {
                                    (name.to_string(), part.to_string())
                                } else {
                                    (name.to_lowercase(), part.to_lowercase())
                                };
                                !asset_name.contains(&part)
                            })
                    })
            })
            .collect();

        if assets.is_empty() {
            return Err(PlatformError::NoMatchingAssets {
                available_assets: release
                    .assets()
                    .into_iter()
                    .map(|a| a.name().to_string())
                    .collect(),
            });
        }

        Ok(assets)
    }

    pub async fn download<A: ReleaseAsset>(
        &self,
        asset: &A,
        options: PlatformDownloadOptions,
    ) -> Result<String, PlatformError> {
        Ok(self
            .downloader
            .download(DownloadOptions {
                url: asset.download_url().to_string(),
                output_path: options.output_path,
                progress_callback: options.progress_callback,
            })
            .await?)
    }
}
