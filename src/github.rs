use serde::Deserialize;

use crate::{
    error::PlatformError,
    platform::{Release, ReleaseAsset, ReleasePlatform},
};

pub struct Github;
impl ReleasePlatform for Github {
    const API_BASE_PRIMARY: &'static str = "https://api.github.com";

    const API_BASE_PKGFORGE: &'static str = "https://api.gh.pkgforge.dev";

    const TOKEN_ENV_VAR: &'static str = "GITHUB_TOKEN";

    fn format_project_path(project: &str) -> Result<(String, String), PlatformError> {
        match project.split_once('/') {
            Some((owner, repo)) if !owner.trim().is_empty() && !repo.trim().is_empty() => {
                Ok((owner.to_string(), repo.to_string()))
            }
            _ => Err(PlatformError::InvalidInput(format!(
                "Github project '{}' must be in 'owner/repo' format",
                project
            ))),
        }
    }

    fn format_api_path(project: &str, tag: Option<&str>) -> Result<String, PlatformError> {
        let (owner, repo) = Self::format_project_path(project)?;
        let base_path = format!("/repos/{}/{}/releases", owner, repo);
        if let Some(tag) = tag {
            Ok(format!("{}/tags/{}?per_page=100", base_path, tag))
        } else {
            Ok(format!("{}?per_page=100", base_path))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GithubRelease {
    name: Option<String>,
    tag_name: String,
    prerelease: bool,
    published_at: String,
    assets: Vec<GithubAsset>,
}

impl Release<GithubAsset> for GithubRelease {
    fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("")
    }

    fn tag_name(&self) -> &str {
        &self.tag_name
    }

    fn is_prerelease(&self) -> bool {
        self.prerelease
    }

    fn published_at(&self) -> &str {
        &self.published_at
    }

    fn assets(&self) -> Vec<GithubAsset> {
        self.assets.clone()
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubAsset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
}

impl ReleaseAsset for GithubAsset {
    fn name(&self) -> &str {
        &self.name
    }

    fn size(&self) -> Option<u64> {
        Some(self.size)
    }

    fn download_url(&self) -> &str {
        &self.browser_download_url
    }
}
