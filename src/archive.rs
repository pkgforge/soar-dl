use tokio::io::AsyncReadExt;
use zip::result::ZipError;

use crate::error::DownloadError;
use std::{
    io::{self, Read},
    path::Path,
};

#[derive(Debug)]
enum ArchiveFormat {
    Zip,
    Gz,
    Xz,
    Bz2,
    Zst,
}

const ZIP_MAGIC_BYTES: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
const GZIP_MAGIC_BYTES: [u8; 2] = [0x1F, 0x8B];
const XZ_MAGIC_BYTES: [u8; 6] = [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00];
const BZIP2_MAGIC_BYTES: [u8; 3] = [0x42, 0x5A, 0x68];
const ZSTD_MAGIC_BYTES: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Extracts the contents of an archive file to a directory.
///
/// This function automatically detects the archive format based on file signatures,
/// then extracts its contents to a directory named after the archive file.
///
/// # Arguments
/// * `path` - Path to the archive file to be extracted
/// * `output_dir` - Path where contents should be extracted
///
/// # Returns
/// * `Ok(())` if extraction was successful
/// * `Err(DownloadError)` if an error occurred during extraction
pub async fn extract_archive<P: AsRef<Path>>(path: P, output_dir: P) -> Result<(), DownloadError> {
    let path = path.as_ref();
    let output_dir = output_dir.as_ref();
    let mut file = tokio::fs::File::open(path).await?;
    let mut magic = vec![0u8; 6];
    let n = file.read(&mut magic).await?;
    let magic = &magic[..n];

    let Some(format) = detect_archive_format(magic) else {
        return Ok(());
    };

    match format {
        ArchiveFormat::Zip => extract_zip(path, output_dir)
            .await
            .map_err(DownloadError::ZipError),
        ArchiveFormat::Gz => extract_tar(path, output_dir, flate2::read::GzDecoder::new).await,
        ArchiveFormat::Xz => extract_tar(path, output_dir, xz2::read::XzDecoder::new).await,
        ArchiveFormat::Bz2 => extract_tar(path, output_dir, bzip2::read::BzDecoder::new).await,
        ArchiveFormat::Zst => {
            extract_tar(path, output_dir, |f| {
                zstd::stream::read::Decoder::new(f).unwrap()
            })
            .await
        }
    }
}

/// Helper function to safely check if a byte slice starts with a pattern
fn starts_with(data: &[u8], pattern: &[u8]) -> bool {
    data.len() >= pattern.len() && &data[..pattern.len()] == pattern
}

/// Detects the archive format by examining the file's magic bytes (signature).
///
/// # Arguments
/// * `magic` - Byte slice containing the beginning of the file (typically first 512 bytes)
///
/// # Returns
/// * `Some(ArchiveFormat)` - The detected archive format
/// * `None` - If the format could not be recognized
fn detect_archive_format(magic: &[u8]) -> Option<ArchiveFormat> {
    if starts_with(magic, &ZIP_MAGIC_BYTES) {
        return Some(ArchiveFormat::Zip);
    }

    if starts_with(magic, &GZIP_MAGIC_BYTES) {
        return Some(ArchiveFormat::Gz);
    }

    if starts_with(magic, &XZ_MAGIC_BYTES) {
        return Some(ArchiveFormat::Xz);
    }

    if starts_with(magic, &BZIP2_MAGIC_BYTES) {
        return Some(ArchiveFormat::Bz2);
    }

    if starts_with(magic, &ZSTD_MAGIC_BYTES) {
        return Some(ArchiveFormat::Zst);
    }

    None
}

/// Generic function for extracting TAR-based archives with different compression formats.
///
/// This function handles the common extraction logic for all TAR-based formats by
/// accepting a decompression function that converts the compressed stream to a
/// readable stream.
///
/// # Arguments
/// * `path` - Path to the archive file
/// * `output_dir` - Path where contents should be extracted
/// * `decompress` - Function that takes a file and returns a decompressed reader
///
/// # Returns
/// * `Ok(())` if extraction was successful
/// * `Err(DownloadError)` if an error occurred
async fn extract_tar<F, R>(
    path: &Path,
    output_dir: &Path,
    decompress: F,
) -> Result<(), DownloadError>
where
    F: FnOnce(std::fs::File) -> R + Send + 'static,
    R: Read + Send + 'static,
{
    let path = path.to_path_buf();
    let output_dir = output_dir.to_path_buf();

    let file = std::fs::File::open(&path)?;
    let decompressed = decompress(file);
    let mut archive = tar::Archive::new(decompressed);
    archive.unpack(&output_dir)?;

    Ok(())
}

/// Extracts a ZIP archive to the specified output directory.
///
/// # Arguments
/// * `path` - Path to the ZIP archive
/// * `output_dir` - Directory where the contents should be extracted
///
/// # Returns
/// * `Ok(())` if extraction was successful
/// * `Err(DownloadError)` if an error occurred
async fn extract_zip(path: &Path, output_dir: &Path) -> Result<(), ZipError> {
    let path = path.to_path_buf();
    let output_dir = output_dir.to_path_buf();

    let file = std::fs::File::open(&path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let out_path = output_dir.join(file.name());

        if file.name().ends_with('/') {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(p) = out_path.parent() {
                if !p.exists() {
                    std::fs::create_dir_all(p)?;
                }
            }
            let mut out_file = std::fs::File::create(&out_path)?;
            io::copy(&mut file, &mut out_file)?;
        }
    }
    Ok(())
}
