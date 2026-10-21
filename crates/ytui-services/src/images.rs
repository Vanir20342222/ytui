use crate::paths::atomic_write;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn thumbnail_decode_cache_and_size_limits_work_without_provider_access() {
        fn png(w: u32, h: u32) -> Vec<u8> {
            let mut bytes = Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(w, h)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            bytes.into_inner()
        }
        let (base, server) = crate::test_http::serve(|target| {
            (
                200,
                "Content-Type: image/png\r\n".into(),
                match target {
                    "/valid.png" => png(160, 90),
                    "/dimensions.png" => png(5000, 1),
                    "/huge.png" => vec![0; 4 * 1024 * 1024 + 1],
                    _ => b"not an image".to_vec(),
                },
            )
        })
        .await;
        let temp = tempfile::tempdir().unwrap();
        let cache = ImageCache::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            temp.path().into(),
        );
        let url = format!("{base}/valid.png");
        let image = cache.load(&url).await.unwrap();
        assert_eq!((image.width(), image.height()), (160, 90));
        for path in ["invalid.png", "dimensions.png", "huge.png"] {
            let url = format!("{base}/{path}");
            assert!(cache.load(&url).await.is_err());
            assert!(!temp.path().join(hex::encode(Sha256::digest(&url))).exists());
        }
        server.abort();
        let _ = server.await;
        assert!(
            cache.load(&url).await.is_ok(),
            "cached thumbnails must survive a service outage"
        );
    }
}

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
            let image = reader.decode()?;
            Ok(if image.width() > 1280 || image.height() > 720 {
                image.thumbnail(1280, 720)
            } else {
                image
            })
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
