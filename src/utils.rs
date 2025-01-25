use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use regex::Regex;
use reqwest::StatusCode;
use tokio::{
    fs::File,
    io::{AsyncReadExt, BufReader},
};

pub const ELF_MAGIC_BYTES: [u8; 4] = [0x7f, 0x45, 0x4c, 0x46];

pub fn extract_filename(url: &str) -> String {
    Path::new(url)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| {
            let dt = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis();
            dt.to_string()
        })
}

pub async fn is_elf<P: AsRef<Path>>(file_path: P) -> bool {
    let Ok(file) = File::open(file_path).await else {
        return false;
    };
    let mut file = BufReader::new(file);

    let mut magic_bytes = [0_u8; 4];
    if file.read_exact(&mut magic_bytes).await.is_ok() {
        return magic_bytes == ELF_MAGIC_BYTES;
    }
    false
}

pub fn should_fallback(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || status.is_server_error()
}

pub fn matches_pattern(
    name: &str,
    regex_patterns: &[Regex],
    match_keywords: &[String],
    exclude_keywords: &[String],
    exact_case: bool,
) -> bool {
    regex_patterns.iter().all(|regex| regex.is_match(name))
        && match_keywords.iter().all(|keyword| {
            keyword
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .all(|part| {
                    let (asset_name, part) = if exact_case {
                        (name.to_string(), part.to_string())
                    } else {
                        (name.to_lowercase(), part.to_lowercase())
                    };
                    asset_name.contains(&part)
                })
        })
        && exclude_keywords.iter().all(|keyword| {
            keyword
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .all(|part| {
                    let (asset_name, part) = if exact_case {
                        (name.to_string(), part.to_string())
                    } else {
                        (name.to_lowercase(), part.to_lowercase())
                    };
                    !asset_name.contains(&part)
                })
        })
}
