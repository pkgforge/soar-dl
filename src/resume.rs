use std::path::{Path, PathBuf};

use reqwest::header::{CONTENT_RANGE, IF_RANGE, RANGE};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::error::DownloadError;

pub struct ResumeSupport;

#[derive(Deserialize, Serialize)]
pub struct DownloadMeta {
    etag: Option<String>,
    last_modified: Option<String>,
}

impl ResumeSupport {
    pub async fn read_metadata<P: AsRef<Path>>(
        meta_path: P,
    ) -> Result<(Option<String>, Option<String>), DownloadError> {
        if fs::try_exists(meta_path.as_ref()).await? {
            let data = fs::read_to_string(meta_path).await?;
            let meta: DownloadMeta =
                serde_json::from_str(&data).map_err(|_| DownloadError::InvalidResponse)?;
            Ok((meta.etag, meta.last_modified))
        } else {
            Ok((None, None))
        }
    }

    pub async fn write_metadata<P: AsRef<Path>>(
        meta_path: P,
        etag: Option<String>,
        last_modified: Option<String>,
    ) -> Result<(), DownloadError> {
        let meta = DownloadMeta {
            etag,
            last_modified,
        };
        fs::write(meta_path, serde_json::to_string(&meta).unwrap()).await?;
        Ok(())
    }

    pub fn get_part_paths<P: AsRef<Path>>(path: P) -> (PathBuf, PathBuf) {
        let path = path.as_ref();
        let part_path = PathBuf::from(&format!("{}.part", path.display()));
        let meta_path = PathBuf::from(&format!("{}.part.meta", path.display()));
        (part_path, meta_path)
    }

    pub fn should_restart_download(
        status: reqwest::StatusCode,
        etag: &Option<String>,
        last_modified: &Option<String>,
        remote_etag: &Option<String>,
        remote_modified: &Option<String>,
    ) -> bool {
        status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE
            || (etag.is_some() && remote_etag.is_some() && etag != remote_etag)
            || (last_modified.is_some()
                && remote_modified.is_some()
                && last_modified != remote_modified)
    }

    pub fn extract_range_info(response: &reqwest::Response, downloaded: u64) -> (bool, u64) {
        let headers = response.headers();
        let should_truncate = headers
            .get(CONTENT_RANGE)
            .and_then(|h| h.to_str().ok())
            .and_then(|r| r.split_whitespace().nth(1))
            .and_then(|range| range.split('/').next())
            .and_then(|s| s.split('-').next())
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|start| start != downloaded);

        let total_size = headers
            .get(CONTENT_RANGE)
            .and_then(|h| h.to_str().ok())
            .and_then(|range| range.rsplit_once('/').and_then(|(_, tot)| tot.parse().ok()))
            .or_else(|| response.content_length())
            .unwrap_or(0);

        (should_truncate, total_size)
    }

    pub fn prepare_resume_headers(
        headers: &mut reqwest::header::HeaderMap,
        downloaded: u64,
        etag: &Option<String>,
        last_modified: &Option<String>,
    ) {
        if downloaded > 0 {
            headers.insert(RANGE, format!("bytes={}-", downloaded).parse().unwrap());

            if let Some(tag) = etag {
                headers.insert(IF_RANGE, tag.parse().unwrap());
            }

            if let Some(modified) = last_modified {
                headers.insert(IF_RANGE, modified.parse().unwrap());
            }
        }
    }
}
