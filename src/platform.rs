use std::{
    env,
    sync::{Arc, LazyLock},
};

use regex::Regex;
use reqwest::header::{HeaderMap, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::{
    downloader::{DownloadOptions, DownloadState, Downloader},
    error::{DownloadError, PlatformError},
    utils::{decode_uri, matches_pattern, should_fallback},
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
    Regex::new(
        r"^(?i)(?:https?://)?(?:github(?:\.com)?[:/])([^/@]+/[^/@]+)(?:@([^/\s]+(?:/[^/\s]*)*)?)?$",
    )
    .unwrap()
});
static GITLAB_RELEASE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?i)(?:https?://)?(?:gitlab(?:\.com)?[:/])((?:\d+)|(?:[^/@]+(?:/[^/@]+)*))(?:@([^/\s]+(?:/[^/\s]*)*)?)?$")
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
                    .map(|tag| tag.as_str().trim_matches(&['\'', '"', ' '][..]))
                    .filter(|&tag| !tag.is_empty())
                    .map(decode_uri);
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
                // if it's API url or contains `/-/` in path, ignore it
                if project.starts_with("api") || project.contains("/-/") {
                    return Ok(PlatformUrl::DirectUrl(url.to_string()));
                }
                let tag = caps
                    .get(2)
                    .map(|tag| tag.as_str().trim_matches(&['\'', '"', ' '][..]))
                    .filter(|&tag| !tag.is_empty())
                    .map(decode_uri);
                if let Some(tag) = tag {
                    return Ok(PlatformUrl::Gitlab(format!("{}@{}", project, tag)));
                } else {
                    return Ok(PlatformUrl::Gitlab(project.to_string()));
                }
            }
            return Err(PlatformError::InvalidInput(url));
        }
        let url = Url::parse(&url).map_err(|_| PlatformError::InvalidInput(url))?;
        Ok(PlatformUrl::DirectUrl(url.to_string()))
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
    fn format_api_path(project: &str, tag: Option<&str>) -> Result<String, PlatformError>;
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
    pub regexes: Vec<Regex>,
    pub globs: Vec<String>,
    pub match_keywords: Vec<String>,
    pub exclude_keywords: Vec<String>,
    pub exact_case: bool,
    pub extract_archive: bool,
    pub extract_dir: Option<String>,
}

#[derive(Default)]
pub struct ReleaseHandler<'a, P: ReleasePlatform> {
    downloader: Downloader<'a>,
    _platform: std::marker::PhantomData<P>,
}

impl<P: ReleasePlatform> ReleaseHandler<'_, P> {
    pub fn new() -> Self {
        Self {
            downloader: Downloader::default(),
            _platform: std::marker::PhantomData,
        }
    }

    async fn call_api(
        &self,
        api_type: &ApiType,
        project: &str,
        tag: Option<&str>,
    ) -> Result<reqwest::Response, PlatformError> {
        let base_url = match api_type {
            ApiType::PkgForge => P::API_BASE_PKGFORGE,
            ApiType::Primary => P::API_BASE_PRIMARY,
        };

        let api_path = P::format_api_path(project, tag)?;
        let url = format!("{}{}", base_url, api_path);

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, "pkgforge/soar".parse().unwrap());

        if matches!(api_type, ApiType::Primary) {
            if let Ok(token) = env::var(P::TOKEN_ENV_VAR) {
                headers.insert(AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());
            }
        }

        Ok(self
            .downloader
            .client()
            .get(&url)
            .headers(headers)
            .send()
            .await
            .map_err(|err| DownloadError::NetworkError { source: err })?)
    }

    pub async fn fetch_releases<R>(
        &self,
        project: &str,
        tag: Option<&str>,
    ) -> Result<Vec<R>, PlatformError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = match self.call_api(&ApiType::PkgForge, project, tag).await {
            Ok(resp) => {
                let status = resp.status();
                if should_fallback(status) {
                    self.call_api(&ApiType::Primary, project, tag).await?
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

        let value: Value = response
            .json()
            .await
            .map_err(|_| PlatformError::InvalidResponse)?;

        match value {
            Value::Array(_) => {
                serde_json::from_value(value).map_err(|_| PlatformError::InvalidResponse)
            }
            Value::Object(_) => {
                let single: R =
                    serde_json::from_value(value).map_err(|_| PlatformError::InvalidResponse)?;
                Ok(vec![single])
            }
            _ => Err(PlatformError::InvalidResponse),
        }
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
                matches_pattern(
                    name,
                    options.regexes.as_slice(),
                    options.globs.as_slice(),
                    options.match_keywords.as_slice(),
                    options.exclude_keywords.as_slice(),
                    options.exact_case,
                )
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
                extract_archive: options.extract_archive,
                extract_dir: options.extract_dir,
            })
            .await?)
    }
}
