use serde::Deserialize;

use crate::{
    error::PlatformError,
    platform::{Release, ReleaseAsset, ReleasePlatform},
};

pub struct Gitlab;
impl ReleasePlatform for Gitlab {
    const API_BASE_PRIMARY: &'static str = "https://gitlab.com";

    const API_BASE_PKGFORGE: &'static str = "https://api.gl.pkgforge.dev";

    const TOKEN_ENV_VAR: &'static str = "GITLAB_TOKEN";

    fn format_project_path(project: &str) -> Result<(String, String), PlatformError> {
        if project.chars().all(|c| c.is_numeric()) {
            Ok((project.to_string(), String::new()))
        } else {
            match project.split_once('/') {
                Some((owner, repo)) => Ok((owner.to_string(), repo.to_string())),
                None => Ok((project.to_string(), String::new())),
            }
        }
    }

    fn format_api_path(project: &str) -> Result<String, PlatformError> {
        if project.chars().all(|c| c.is_numeric()) {
            Ok(format!("/api/v4/projects/{}/releases", project))
        } else {
            let encoded_path = project.replace('/', "%2F");
            Ok(format!("/api/v4/projects/{}/releases", encoded_path))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GitlabAssets {
    pub links: Vec<GitlabAsset>,
}

#[derive(Debug, Deserialize)]
pub struct GitlabRelease {
    name: String,
    tag_name: String,
    upcoming_release: bool,
    released_at: String,
    assets: GitlabAssets,
}

impl Release<GitlabAsset> for GitlabRelease {
    fn name(&self) -> &str {
        &self.name
    }

    fn tag_name(&self) -> &str {
        &self.tag_name
    }

    fn is_prerelease(&self) -> bool {
        self.upcoming_release
    }

    fn published_at(&self) -> &str {
        &self.released_at
    }

    fn assets(&self) -> Vec<GitlabAsset> {
        self.assets.links.clone()
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitlabAsset {
    pub name: String,
    pub direct_asset_url: String,
}

impl ReleaseAsset for GitlabAsset {
    fn name(&self) -> &str {
        &self.name
    }

    fn size(&self) -> Option<u64> {
        None
    }

    fn download_url(&self) -> &str {
        &self.direct_asset_url
    }
}
