//! Size-aware thumbnail cache. No cookies are sent to image origins.
use crate::paths::atomic_write;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Memory {
    images: HashMap<String, (u64, image::DynamicImage)>,
    clock: u64,
}
impl Memory {
    fn get(&mut self, key: &str) -> Option<image::DynamicImage> {
        self.clock += 1;
        self.images.get_mut(key).map(|(age, image)| {
            *age = self.clock;
            image.clone()
        })
    }
    fn insert(&mut self, key: String, image: image::DynamicImage) {
        self.clock += 1;
        self.images.insert(key, (self.clock, image));
        while self.images.len() > 96
            || self
                .images
                .values()
                .map(|(_, i)| i.as_bytes().len())
                .sum::<usize>()
                > 32 * 1024 * 1024
        {
            let Some(oldest) = self
                .images
                .iter()
                .min_by_key(|(_, (age, _))| age)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.images.remove(&oldest);
        }
    }
}
#[derive(Clone)]
pub struct ImageCache {
    client: reqwest::Client,
    dir: PathBuf,
    permits: Arc<tokio::sync::Semaphore>,
    memory: Arc<Mutex<Memory>>,
    flights: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
    writes: Arc<AtomicUsize>,
}
#[derive(Clone, Debug, Default)]
pub struct CacheStats {
    pub files: usize,
    pub disk_bytes: u64,
    pub memory_images: usize,
    pub memory_bytes: usize,
}
fn owned_cache_file(entry: &std::fs::DirEntry) -> bool {
    let name = entry.file_name();
    let name = name.to_string_lossy();
    name.len() == 64
        && name.bytes().all(|b| b.is_ascii_hexdigit())
        && entry.file_type().is_ok_and(|kind| kind.is_file())
}
impl ImageCache {
    pub async fn inspect(&self, clear: bool) -> Result<CacheStats> {
        // Wait for current fetch/write operations and prevent new writes while
        // clearing. The semaphore is shared by every clone of this cache.
        let _permit = self.permits.acquire_many(4).await?;
        let dir = self.dir.clone();
        let mut stats = tokio::task::spawn_blocking(move || {
            let mut stats = CacheStats::default();
            match std::fs::symlink_metadata(&dir) {
                Ok(metadata) => ensure!(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    "Cache directory must not be a symlink"
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(stats),
                Err(error) => return Err(error.into()),
            }
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                if !owned_cache_file(&entry) {
                    continue;
                }
                let metadata = entry.metadata()?;
                if clear {
                    std::fs::remove_file(entry.path())?;
                }
                stats.files += 1;
                stats.disk_bytes = stats.disk_bytes.saturating_add(metadata.len());
            }
            Ok::<_, anyhow::Error>(stats)
        })
        .await??;
        let mut memory = self.memory.lock().unwrap();
        stats.memory_images = memory.images.len();
        stats.memory_bytes = memory
            .images
            .values()
            .map(|(_, image)| image.as_bytes().len())
            .sum();
        if clear {
            *memory = Memory::default();
        }
        Ok(stats)
    }
    pub fn new(client: reqwest::Client, dir: PathBuf) -> Self {
        Self {
            client,
            dir,
            permits: Arc::new(tokio::sync::Semaphore::new(4)),
            memory: Arc::new(Mutex::new(Memory::default())),
            flights: Arc::new(Mutex::new(HashMap::new())),
            writes: Arc::new(AtomicUsize::new(0)),
        }
    }
    pub async fn load(&self, url: &str) -> Result<image::DynamicImage> {
        self.load_sized(url, 1280, 720).await
    }
    pub async fn load_sized(
        &self,
        url: &str,
        width: u32,
        height: u32,
    ) -> Result<image::DynamicImage> {
        ytui_core::http_url(url)?;
        let (width, height) = (width.clamp(1, 1280), height.clamp(1, 720));
        let key = hex::encode(Sha256::digest(format!("v2:{url}:{width}:{height}")));
        if let Some(image) = self.memory.lock().unwrap().get(&key) {
            return Ok(image);
        }
        let flight = {
            let mut flights = self.flights.lock().unwrap();
            flights.retain(|_, v| v.strong_count() > 0);
            if let Some(flight) = flights.get(&key).and_then(Weak::upgrade) {
                flight
            } else {
                let flight = Arc::new(tokio::sync::Mutex::new(()));
                flights.insert(key.clone(), Arc::downgrade(&flight));
                flight
            }
        };
        let _flight = flight.lock().await;
        if let Some(image) = self.memory.lock().unwrap().get(&key) {
            return Ok(image);
        }
        let _permit = self.permits.acquire().await?;
        let path = self.dir.join(&key);
        if let Ok(bytes) = tokio::fs::read(&path).await {
            if let Ok(image) = decode(bytes, width, height).await {
                self.memory.lock().unwrap().insert(key, image.clone());
                return Ok(image);
            }
            let _ = tokio::fs::remove_file(&path).await;
        }
        let mut response = crate::http::get(
            ytui_core::http_url(url)?,
            |url| {
                Ok(self
                    .client
                    .get(url.clone())
                    .timeout(std::time::Duration::from_secs(8)))
            },
            crate::http::Redirects::Public,
            "Thumbnail",
        )
        .await
        .context("Thumbnail unavailable")?;
        ensure!(
            response.status().is_success(),
            "Thumbnail HTTP {}",
            response.status().as_u16()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 4 * 1024 * 1024,
                "Thumbnail too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        let image = decode(bytes, width, height).await?;
        // Displaying a valid image must not depend on an optional disk cache.
        self.memory.lock().unwrap().insert(key, image.clone());
        let saved = image.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let mut bytes = Cursor::new(Vec::new());
            saved.write_to(&mut bytes, image::ImageFormat::Png)?;
            atomic_write(&path, bytes.get_ref())
        })
        .await;
        // A directory scan on every cache hit made navigating large feeds expensive.
        if self
            .writes
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(32)
        {
            let _ = self.prune().await;
        }
        Ok(image)
    }
    pub async fn prune(&self) -> Result<()> {
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&dir)?;
            let mut files = std::fs::read_dir(dir)?
                .flatten()
                .filter(owned_cache_file)
                .filter_map(|e| {
                    let m = e.metadata().ok()?;
                    m.is_file()
                        .then(|| Some((m.modified().ok()?, m.len(), e.path())))?
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
async fn decode(bytes: Vec<u8>, width: u32, height: u32) -> Result<image::DynamicImage> {
    tokio::task::spawn_blocking(move || {
        let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode()?;
        Ok(if image.width() > width || image.height() > height {
            image.thumbnail(width, height)
        } else {
            image
        })
    })
    .await?
}

/// Prefer the smallest advertised image that covers the displayed pixels.
/// Fall back to the largest advertised image; never manufacture a missing CDN variant.
pub fn thumbnail_url(media: &ytui_core::Media, width: u32, height: u32) -> String {
    let selected = media
        .thumbnails
        .iter()
        .filter(|t| t.width >= width && t.height >= height)
        .min_by_key(|t| t.width as u64 * t.height as u64)
        .or_else(|| {
            media
                .thumbnails
                .iter()
                .max_by_key(|t| t.width as u64 * t.height as u64)
        });
    let mut url = selected
        .map(|t| t.url.clone())
        .unwrap_or_else(|| media.thumbnail.clone());
    if let Ok(mut parsed) = url::Url::parse(&url) {
        let host = parsed.host_str().unwrap_or_default();
        if host == "lh3.googleusercontent.com"
            || host.ends_with(".googleusercontent.com")
            || host.ends_with(".ggpht.com")
        {
            // These image endpoints support dynamic size parameters. Preserve all other URLs.
            let path = parsed
                .path()
                .split_once('=')
                .map(|(p, _)| p)
                .unwrap_or(parsed.path())
                .to_owned();
            parsed.set_path(&format!(
                "{path}=w{}-h{}-l90-rj",
                width.clamp(1, 1280),
                height.clamp(1, 720)
            ));
            url = parsed.into();
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cache_inspection_and_clear_preserve_foreign_files_and_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("images");
        std::fs::create_dir(&dir).unwrap();
        let key = "a".repeat(64);
        std::fs::write(dir.join(&key), b"cached").unwrap();
        std::fs::write(dir.join("my-video.mp4"), b"original").unwrap();
        let outside = temp.path().join("outside");
        std::fs::write(&outside, b"original").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, dir.join("b".repeat(64))).unwrap();
        let cache = ImageCache::new(crate::http_client(None).unwrap(), dir.clone());
        assert_eq!(cache.inspect(false).await.unwrap().files, 1);
        assert_eq!(cache.inspect(true).await.unwrap().disk_bytes, 6);
        assert!(!dir.join(key).exists());
        assert_eq!(
            std::fs::read(dir.join("my-video.mp4")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read(outside).unwrap(), b"original");
        #[cfg(unix)]
        {
            let link = temp.path().join("linked-cache");
            std::os::unix::fs::symlink(&dir, &link).unwrap();
            assert!(
                ImageCache::new(crate::http_client(None).unwrap(), link)
                    .inspect(true)
                    .await
                    .is_err()
            );
        }
    }
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

#[cfg(test)]
mod sized_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[tokio::test]
    async fn concurrent_requests_share_one_fetch_and_resized_disk_cache_survives_restart() {
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let (base, server) = crate::test_http::serve_request(move |request| {
            assert!(!request.to_lowercase().contains("\ncookie:"));
            calls.fetch_add(1, Ordering::SeqCst);
            let mut bytes = Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(1280, 720)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            (
                200,
                "Content-Type: image/png\r\n".into(),
                bytes.into_inner(),
            )
        })
        .await;
        let temp = tempfile::tempdir().unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let cache = ImageCache::new(client.clone(), temp.path().into());
        let url = format!("{base}/image.png");
        let (a, b) = tokio::join!(
            cache.load_sized(&url, 320, 180),
            cache.load_sized(&url, 320, 180)
        );
        for image in [a.unwrap(), b.unwrap()] {
            assert_eq!((image.width(), image.height()), (320, 180));
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        server.abort();
        let _ = server.await;
        drop(cache);
        let cache = ImageCache::new(client, temp.path().into());
        let cached = cache.load_sized(&url, 320, 180).await.unwrap();
        assert_eq!((cached.width(), cached.height()), (320, 180));
        let media = ytui_core::Media {
            thumbnail: "https://cdn.test/large".into(),
            thumbnails: vec![
                ytui_core::Thumbnail {
                    url: "https://cdn.test/small".into(),
                    width: 120,
                    height: 90,
                },
                ytui_core::Thumbnail {
                    url: "https://cdn.test/medium".into(),
                    width: 480,
                    height: 270,
                },
                ytui_core::Thumbnail {
                    url: "https://cdn.test/large".into(),
                    width: 1280,
                    height: 720,
                },
            ],
            ..Default::default()
        };
        assert_eq!(thumbnail_url(&media, 320, 180), "https://cdn.test/medium");
        assert_eq!(thumbnail_url(&media, 900, 500), "https://cdn.test/large");
    }
}
