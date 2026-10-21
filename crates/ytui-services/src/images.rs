use crate::paths::atomic_write;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};

#[derive(Clone)]
pub struct ImageCache {
    client: reqwest::Client,
    dir: PathBuf,
    permits: std::sync::Arc<tokio::sync::Semaphore>,
}
impl ImageCache {
    pub fn new(client: reqwest::Client, dir: PathBuf) -> Self {
        Self {
            client,
            dir,
            permits: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }
    pub async fn load(&self, url: &str) -> Result<image::DynamicImage> {
        let _permit = self.permits.acquire().await?;
        ytui_core::http_url(url)?;
        let filename = hex::encode(Sha256::digest(url));
        let path = self.dir.join(filename);
        let bytes = if let Ok(bytes) = tokio::fs::read(&path).await {
            bytes
        } else {
            let mut response = self
                .client
                .get(url)
                .send()
                .await
                .context("Thumbnail unavailable")?;
            ensure!(response.status().is_success(), "Thumbnail unavailable");
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                ensure!(
                    bytes.len() + chunk.len() <= 4 * 1024 * 1024,
                    "Thumbnail too large"
                );
                bytes.extend_from_slice(&chunk);
            }
            let saved = bytes.clone();
            let p = path.clone();
            tokio::task::spawn_blocking(move || atomic_write(&p, &saved)).await??;
            bytes
        };
        let decoded = tokio::task::spawn_blocking(move || {
            let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(4096);
            limits.max_image_height = Some(4096);
            limits.max_alloc = Some(64 * 1024 * 1024);
            reader.limits(limits);
            Ok(reader.decode()?.thumbnail(1280, 720))
        })
        .await?;
        if decoded.is_err() {
            let _ = tokio::fs::remove_file(&path).await;
        } else {
            let _ = self.prune().await;
        }
        decoded
    }
    pub async fn prune(&self) -> Result<()> {
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&dir)?;
            let mut files = std::fs::read_dir(dir)?
                .flatten()
                .filter_map(|e| {
                    let m = e.metadata().ok()?;
                    if m.is_file() {
                        Some((m.modified().ok()?, m.len(), e.path()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            files.sort_by_key(|(time, _, _)| *time);
            let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
            for (_, len, path) in files {
                if total <= 128 * 1024 * 1024 {
                    break;
                }
                std::fs::remove_file(path)?;
                total = total.saturating_sub(len);
            }
            Ok::<_, anyhow::Error>(())
        })
        .await?
    }
}
