//! Memory-only stream preparation. No signed URL or account data is persisted.
use crate::{auth::Session, cache::ResponseCache, native::Resolved, paths::Paths};
use anyhow::Result;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use ytui_core::{Config, DownloadOptions, Job, Media};

pub(crate) type Streams = Arc<ResponseCache<Resolved>>;

pub(crate) fn key(config: &Config, media: &Media, options: &DownloadOptions) -> Result<String> {
    let source = crate::sponsorblock::video_id(media)
        .map(|id| format!("https://www.youtube.com/watch?v={id}"))
        .unwrap_or_else(|| media.url.clone());
    Ok(serde_json::to_string(&(
        source,
        options.audio,
        &options.quality,
        &config.proxy,
        &config.native_js_runtime,
        &config.ffmpeg,
        &options.format_selector,
    ))?)
}

pub(crate) fn ttl_at(stream: &Resolved, now: u64) -> Duration {
    let mut ttl = Duration::from_secs(300);
    for raw in [
        &stream.url,
        stream.audio_url.as_ref().unwrap_or(&stream.url),
    ] {
        let Ok(url) = url::Url::parse(raw) else {
            continue;
        };
        let mut expiries = url
            .query_pairs()
            .filter(|(name, _)| matches!(name.as_ref(), "expire" | "expires"))
            .filter_map(|(_, value)| value.parse::<u64>().ok())
            .collect::<Vec<_>>();
        let segments = url
            .path_segments()
            .map(|segments| segments.collect::<Vec<_>>())
            .unwrap_or_default();
        expiries.extend(
            segments
                .windows(2)
                .filter(|pair| matches!(pair[0], "expire" | "expires"))
                .filter_map(|pair| pair[1].parse::<u64>().ok()),
        );
        for expiry in expiries {
            ttl = ttl.min(Duration::from_secs(
                expiry.saturating_sub(now.saturating_add(30)),
            ));
        }
    }
    ttl
}

pub(crate) fn ttl(stream: &Resolved) -> Duration {
    ttl_at(
        stream,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}

pub(crate) async fn resolve(
    streams: &Streams,
    request: crate::native::WorkerRequest,
    dir: PathBuf,
    cancel: CancellationToken,
) -> Result<Resolved> {
    let key = key(&request.config, &request.job.media, &request.job.options)?;
    if streams
        .get(&key)
        .is_some_and(|stream| ttl(&stream).is_zero())
    {
        streams.invalidate(&key);
    }
    streams
        .load_with_ttl(key, ttl, || crate::native::resolve(request, &dir, cancel))
        .await
}

pub(crate) struct Preparation {
    pub key: String,
    pub cancel: CancellationToken,
    task: Option<JoinHandle<Result<Resolved>>>,
    cancel_on_drop: bool,
}
impl Preparation {
    pub fn spawn<F, Fut>(key: String, fetch: F) -> Self
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: std::future::Future<Output = Result<Resolved>> + Send + 'static,
    {
        let cancel = CancellationToken::new();
        let task = tokio::spawn(fetch(cancel.clone()));
        Self {
            key,
            cancel,
            task: Some(task),
            cancel_on_drop: true,
        }
    }
    pub fn begin(
        config: Config,
        paths: Paths,
        session: Option<Arc<Session>>,
        media: Media,
        audio: bool,
        streams: Streams,
    ) -> Result<Self> {
        let mut options = config.download.clone();
        options.audio = audio;
        options.format_selector = None;
        let key = key(&config, &media, &options)?;
        Ok(Self::spawn(key, move |cancel| async move {
            // The owned export lives until the supervised extractor is reaped.
            let cookies = session
                .as_ref()
                .map(|s| {
                    s.headers("https://www.youtube.com")?;
                    s.export(&paths.state.join("auth/transient"))
                })
                .transpose()?;
            resolve(
                &streams,
                crate::native::WorkerRequest {
                    job: Job::new(media, options),
                    config,
                    cookies: cookies.as_ref().map(|f| f.path().to_owned()),
                    resolve: true,
                    staging: None,
                    player_clients: None,
                },
                paths.state.join("auth/transient"),
                cancel,
            )
            .await
        }))
    }
    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(|task| task.is_finished())
    }
    pub async fn finish(mut self) -> Result<Resolved> {
        self.cancel_on_drop = false;
        self.task.take().expect("preparation task").await?
    }
    pub async fn stop(mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for Preparation {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancel.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stream(url: &str, audio: Option<&str>) -> Resolved {
        Resolved {
            url: url.into(),
            audio_url: audio.map(str::to_owned),
            headers: vec![],
            catalog: Default::default(),
        }
    }
    #[test]
    fn aliases_share_streams_but_audio_quality_and_transport_do_not() {
        let config = Config::default();
        let mut options = config.download.clone();
        options.audio = true;
        let media = |url: &str| Media {
            url: url.into(),
            ..Default::default()
        };
        let canonical = media("https://www.youtube.com/watch?v=abcdefghijk");
        let expected = key(&config, &canonical, &options).unwrap();
        for url in [
            "https://music.youtube.com/watch?v=abcdefghijk",
            "https://youtu.be/abcdefghijk?t=12",
            "https://www.youtube.com/watch?v=abcdefghijk&list=context",
        ] {
            assert_eq!(key(&config, &media(url), &options).unwrap(), expected);
        }
        options.audio = false;
        assert_ne!(key(&config, &canonical, &options).unwrap(), expected);
        options.audio = true;
        options.quality = "144".into();
        assert_ne!(key(&config, &canonical, &options).unwrap(), expected);
        options = config.download.clone();
        options.audio = true;
        let proxy = Config {
            proxy: Some("http://proxy.invalid".into()),
            ..config.clone()
        };
        assert_ne!(key(&proxy, &canonical, &options).unwrap(), expected);
    }
    #[test]
    fn signed_expiry_bounds_both_streams_with_a_safety_margin() {
        assert_eq!(
            ttl_at(
                &stream(
                    "https://manifest.googlevideo.com/api/manifest/hls/expire/1060/other/value",
                    None
                ),
                1000
            ),
            Duration::from_secs(30)
        );
        assert_eq!(
            ttl_at(
                &stream(
                    "https://cdn.invalid/video?expire=1100",
                    Some("https://cdn.invalid/audio?expire=1060")
                ),
                1000
            ),
            Duration::from_secs(30)
        );
        assert_eq!(
            ttl_at(&stream("https://cdn.invalid/video?expire=1029", None), 1000),
            Duration::ZERO
        );
        assert_eq!(
            ttl_at(&stream("https://cdn.invalid/video?expire=999", None), 1000),
            Duration::ZERO
        );
        assert_eq!(
            ttl_at(&stream("https://cdn.invalid/video", None), 1000),
            Duration::from_secs(300)
        );
    }
    #[tokio::test]
    async fn dynamic_expiry_does_not_cache_a_nearly_expired_resolution() {
        let cache = ResponseCache::new(2, 4096);
        cache
            .load_with_ttl(
                "expired".into(),
                |s| ttl_at(s, 1000),
                || async { Ok(stream("https://cdn.invalid/video?expire=1001", None)) },
            )
            .await
            .unwrap();
        assert!(cache.get("expired").is_none());
    }
    #[tokio::test]
    async fn stopping_or_dropping_preparation_cancels_owned_work() {
        for drop_only in [false, true] {
            let began = Arc::new(tokio::sync::Notify::new());
            let ended = Arc::new(tokio::sync::Notify::new());
            let (a, b) = (began.clone(), ended.clone());
            let preparation = Preparation::spawn("fixture".into(), move |cancel| async move {
                a.notify_one();
                cancel.cancelled().await;
                b.notify_one();
                anyhow::bail!("cancelled")
            });
            began.notified().await;
            if drop_only {
                drop(preparation);
            } else {
                preparation.stop().await;
            }
            tokio::time::timeout(Duration::from_secs(1), ended.notified())
                .await
                .unwrap();
        }
    }
}
