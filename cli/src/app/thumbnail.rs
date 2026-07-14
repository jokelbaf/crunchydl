//! Bounded on-disk thumbnail cache and resource-limited image decoding.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use futures_util::StreamExt;
use image::ImageReader;
use sha2::{Digest, Sha256};

const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CACHE_FILES: usize = 256;
const MAX_DIMENSION: u32 = 4096;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) async fn load(
    client: &reqwest::Client,
    cache_root: &Path,
    source: &str,
) -> std::result::Result<image::DynamicImage, ()> {
    let path = cache_root.join(cache_key(source));
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let bytes = fetch(client, source).await?;
            let cached = bytes.clone();
            let root = cache_root.to_path_buf();
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let _ = store_and_prune(&root, &path, &cached);
            })
            .await
            .map_err(|_| ())?;
            bytes
        }
        Err(_) => return Err(()),
    };
    tokio::task::spawn_blocking(move || decode(&bytes))
        .await
        .map_err(|_| ())?
}

async fn fetch(client: &reqwest::Client, source: &str) -> std::result::Result<Vec<u8>, ()> {
    let response = client.get(source).send().await.map_err(|_| ())?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ())?;
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn decode(bytes: &[u8]) -> std::result::Result<image::DynamicImage, ()> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| ())?;
    Ok(image.thumbnail(1600, 1600))
}

fn store_and_prune(root: &Path, path: &Path, bytes: &[u8]) -> std::result::Result<(), ()> {
    std::fs::create_dir_all(root).map_err(|_| ())?;
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes).map_err(|_| ())?;
    std::fs::rename(&temporary, path).map_err(|_| ())?;
    prune(root)
}

fn prune(root: &Path) -> std::result::Result<(), ()> {
    let mut files = std::fs::read_dir(root)
        .map_err(|_| ())?
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then_some((
                entry.path(),
                metadata.len(),
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|(_, _, modified)| *modified);
    let mut bytes = files.iter().map(|(_, length, _)| *length).sum::<u64>();
    let mut count = files.len();
    for (path, length, _) in files {
        if count <= MAX_CACHE_FILES && bytes <= MAX_CACHE_BYTES {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            count -= 1;
            bytes = bytes.saturating_sub(length);
        }
    }
    Ok(())
}

fn cache_key(source: &str) -> PathBuf {
    let digest = Sha256::digest(source.as_bytes());
    let mut name = String::with_capacity(digest.len() * 2 + 4);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(name, "{byte:02x}");
    }
    name.push_str(".img");
    PathBuf::from(name)
}
