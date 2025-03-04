use std::{error::Error, fmt::Display, io};

#[derive(Debug)]
pub enum DownloadError {
    InvalidUrl {
        url: String,
        source: url::ParseError,
    },
    IoError(io::Error),
    NetworkError {
        source: reqwest::Error,
    },
    ResourceError {
        url: String,
        status: reqwest::StatusCode,
    },
    InvalidResponse,
    LayersNotFound,
    ChunkError,
    FileNameNotFound,
}

impl Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::IoError(err) => write!(f, "IO error: {}", err),
            DownloadError::InvalidUrl { url, .. } => write!(f, "Invalid URL: {}", url),
            DownloadError::NetworkError { .. } => write!(f, "Network Request failed"),
            DownloadError::ResourceError { url, status } => {
                write!(f, "Failed to fetch resource from {} [{}]", url, status)
            }
            DownloadError::LayersNotFound => write!(f, "No downloadable layers found"),
            DownloadError::InvalidResponse => write!(f, "Failed to parse response"),
            DownloadError::ChunkError => write!(f, "Failed to read chunk"),
            DownloadError::FileNameNotFound => {
                write!(
                    f,
                    "Couldn't find filename. Please provide filename explicitly."
                )
            }
        }
    }
}

impl Error for DownloadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            DownloadError::IoError(err) => Some(err),
            DownloadError::InvalidUrl { source, .. } => Some(source),
            DownloadError::NetworkError { source } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for DownloadError {
    fn from(value: io::Error) -> Self {
        Self::IoError(value)
    }
}

#[derive(Debug)]
pub enum PlatformError {
    ApiError { status: reqwest::StatusCode },
    DownloadError(DownloadError),
    InvalidInput(String),
    InvalidResponse,
    NoMatchingAssets { available_assets: Vec<String> },
    NoRelease { tag: Option<String> },
    RepositoryNotFound { owner: String, repo: String },
}

impl Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformError::ApiError { status } => write!(f, "API error [{}]", status),
            PlatformError::DownloadError(err) => write!(f, "Download error: {}", err),
            PlatformError::InvalidInput(msg) => {
                write!(f, "{} is invalid. Should be in format (owner/repo)", msg)
            }
            PlatformError::InvalidResponse => write!(f, "Failed to parse response"),
            PlatformError::NoRelease { tag } => write!(
                f,
                "No {} found.",
                tag.clone()
                    .map(|t| format!("tag {}", t))
                    .unwrap_or("release".to_string())
            ),
            PlatformError::NoMatchingAssets { .. } => write!(f, "No matching assets found"),
            PlatformError::RepositoryNotFound { owner, repo } => {
                write!(f, "Repository not found: {}/{}", owner, repo)
            }
        }
    }
}

impl From<DownloadError> for PlatformError {
    fn from(value: DownloadError) -> Self {
        Self::DownloadError(value)
    }
}
