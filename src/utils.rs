use std::{
    env,
    path::{Path, PathBuf},
};

use fast_glob::glob_match;
use regex::Regex;
use reqwest::StatusCode;
use tokio::{
    fs::File,
    io::{AsyncReadExt, BufReader},
};
use url::Url;

pub const ELF_MAGIC_BYTES: [u8; 4] = [0x7f, 0x45, 0x4c, 0x46];

pub fn extract_filename_from_url(url: &str) -> Option<String> {
    let url = Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| url.to_string());
    Path::new(&url)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
}

pub fn extract_filename(header_value: &str) -> Option<String> {
    header_value.split(';').find_map(|part| {
        let part = part.trim();
        if part.starts_with("filename=") {
            Some(
                part.trim_start_matches("filename=")
                    .trim_matches('"')
                    .to_string(),
            )
        } else {
            None
        }
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
    regexes: &[Regex],
    globs: &[String],
    match_keywords: &[String],
    exclude_keywords: &[String],
    exact_case: bool,
) -> bool {
    regexes.iter().all(|regex| regex.is_match(name))
        && globs.iter().all(|glob| glob_match(glob, name))
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

// https://users.rust-lang.org/t/encode-decode-uri/90017/16
pub fn decode_uri(s: impl AsRef<str>) -> String {
    let re = Regex::new(r"(%[A-Fa-f0-9]{2})+").unwrap();

    re.replace_all(s.as_ref(), |caps: &regex::Captures| {
        let seq = caps.get(0).map_or("", |m| m.as_str());
        let mut r = Vec::<u8>::new();
        let inp: Vec<u8> = seq.bytes().collect();
        let mut i: usize = 0;
        while i != inp.len() {
            r.push(
                u8::from_str_radix(&String::from_utf8_lossy(&[inp[i + 1], inp[i + 2]]), 16)
                    .unwrap_or(0),
            );
            i += 3;
        }
        String::from_utf8_lossy(&r).into_owned()
    })
    .into_owned()
}

pub fn build_absolute_path<P: AsRef<Path>>(path: P) -> std::io::Result<PathBuf> {
    let path = path.as_ref();
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir()?.join(path))
    }
}
