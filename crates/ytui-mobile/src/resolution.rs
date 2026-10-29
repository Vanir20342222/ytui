//! In-memory, bounded stream leases. Never persist signed URLs or reuse across accounts/networks.
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub epoch: u64,
    pub account: u64,
    pub account_index: u32,
    pub proxy: Option<String>,
    pub url: String,
    pub audio: bool,
    pub quality: String,
}

struct Entry {
    value: Value,
    until: Instant,
    touched: Instant,
    bytes: usize,
}
#[derive(Default)]
pub(crate) struct Resolver {
    epoch: AtomicU64,
    entries: Mutex<HashMap<Key, Entry>>,
    flights: Mutex<HashMap<Key, Weak<tokio::sync::Mutex<()>>>>,
    client: Mutex<Option<(Key, Arc<yt_dlp_rs::YoutubeDl>)>>,
}
impl Resolver {
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
    pub fn clear(&self) {
        let mut client = self.client.lock().expect("extractor client");
        let mut entries = self.entries.lock().expect("resolution cache");
        self.epoch.fetch_add(1, Ordering::AcqRel);
        entries.clear();
        *client = None;
    }
    pub fn client(
        &self,
        key: &Key,
        create: impl FnOnce() -> Result<yt_dlp_rs::YoutubeDl>,
    ) -> Result<Arc<yt_dlp_rs::YoutubeDl>> {
        let mut cache = self.client.lock().expect("extractor client");
        if let Some((scope, client)) = cache.as_ref()
            && scope.epoch == key.epoch
            && scope.account == key.account
            && scope.account_index == key.account_index
            && scope.proxy == key.proxy
        {
            return Ok(client.clone());
        }
        let client = Arc::new(create()?);
        ensure!(
            key.epoch == self.epoch(),
            "Session or network changed during playback preparation"
        );
        *cache = Some((key.clone(), client.clone()));
        Ok(client)
    }
    pub async fn get_or_resolve(
        &self,
        key: Key,
        fresh: bool,
        load: impl Future<Output = Result<Value>>,
    ) -> Result<Value> {
        let flight = {
            let mut flights = self.flights.lock().expect("resolution flights");
            flights.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = flights.get(&key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                flights.insert(key.clone(), Arc::downgrade(&lock));
                lock
            }
        };
        let _guard = flight.lock().await;
        ensure!(
            key.epoch == self.epoch(),
            "Session or network changed during playback preparation"
        );
        {
            let mut cache = self.entries.lock().expect("resolution cache");
            ensure!(
                key.epoch == self.epoch(),
                "Session or network changed during playback preparation"
            );
            cache.retain(|_, entry| entry.until > Instant::now());
            if !fresh && let Some(entry) = cache.get_mut(&key) {
                entry.touched = Instant::now();
                return Ok(entry.value.clone());
            }
            if fresh {
                cache.remove(&key);
            }
        }
        let value = load.await?;
        ensure!(
            key.epoch == self.epoch(),
            "Session or network changed during playback preparation"
        );
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Some(ttl) = lease_seconds(&value, now) {
            let bytes = value.to_string().len();
            if bytes <= 1024 * 1024 {
                let mut cache = self.entries.lock().expect("resolution cache");
                ensure!(
                    key.epoch == self.epoch(),
                    "Session or network changed during playback preparation"
                );
                let now = Instant::now();
                cache.insert(
                    key,
                    Entry {
                        value: value.clone(),
                        until: now + Duration::from_secs(ttl),
                        touched: now,
                        bytes,
                    },
                );
                while cache.len() > 24
                    || cache.values().map(|e| e.bytes).sum::<usize>() > 8 * 1024 * 1024
                {
                    let Some(oldest) = cache
                        .iter()
                        .min_by_key(|(_, e)| e.touched)
                        .map(|(k, _)| k.clone())
                    else {
                        break;
                    };
                    cache.remove(&oldest);
                }
            }
        }
        Ok(value)
    }
}

fn lease_seconds(value: &Value, now: u64) -> Option<u64> {
    if value["live"].as_bool() == Some(true) {
        return None;
    }
    value["url"].as_str()?;
    let mut ttl = 300;
    for field in ["url", "audio_url"] {
        if let Some(raw) = value[field].as_str() {
            let url = url::Url::parse(raw).ok()?;
            for (_, expiration) in url
                .query_pairs()
                .filter(|(k, _)| k == "expire" || k == "expires")
            {
                let remaining = expiration
                    .parse::<u64>()
                    .ok()?
                    .checked_sub(now.checked_add(45)?)?;
                ttl = ttl.min(remaining);
            }
        }
    }
    (ttl > 0).then_some(ttl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn key() -> Key {
        Key {
            epoch: 0,
            account: 0,
            account_index: 0,
            proxy: None,
            url: "https://youtube.test/watch?v=fixture".into(),
            audio: true,
            quality: "1080".into(),
        }
    }
    #[test]
    fn leases_respect_both_stream_expirations_and_live_exclusion() {
        assert_eq!(
            lease_seconds(
                &json!({"url":"https://cdn.test/v?expire=1200","audio_url":"https://cdn.test/a?expire=1100"}),
                1000
            ),
            Some(55)
        );
        assert_eq!(
            lease_seconds(&json!({"url":"https://cdn.test/v?expire=1040"}), 1000),
            None
        );
        assert_eq!(
            lease_seconds(&json!({"url":"https://cdn.test/v?expire=invalid"}), 1000),
            None
        );
        assert_eq!(
            lease_seconds(&json!({"url":"https://cdn.test/v","live":true}), 1000),
            None
        );
    }
    #[tokio::test]
    async fn concurrent_resolution_deduplicates_and_scope_or_fresh_requests_do_not_reuse() {
        let cache = Resolver::default();
        let count = AtomicU64::new(0);
        let load = || async {
            count.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            Ok(json!({"url":"https://cdn.test/media"}))
        };
        let (a, b) = tokio::join!(
            cache.get_or_resolve(key(), false, load()),
            cache.get_or_resolve(key(), false, load())
        );
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let mut other = key();
        other.account = 1;
        cache.get_or_resolve(other, false, load()).await.unwrap();
        cache.get_or_resolve(key(), true, load()).await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
        cache.clear();
        assert!(cache.get_or_resolve(key(), false, load()).await.is_err());
        let mut current = key();
        current.epoch = cache.epoch();
        cache.get_or_resolve(current, false, load()).await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }
    #[tokio::test]
    async fn invalidation_during_extraction_never_caches_or_returns_old_session_streams() {
        let cache = Resolver::default();
        let result = cache
            .get_or_resolve(key(), false, async {
                cache.clear();
                Ok(json!({"url":"https://cdn.test/private"}))
            })
            .await;
        assert!(result.is_err());
        assert!(cache.entries.lock().unwrap().is_empty());
    }
}
