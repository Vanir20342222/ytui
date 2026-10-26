//! Bounded, process-local response cache with cancellation-safe request sharing.
//! Each provider owns its cache, so accounts and network configurations stay isolated.
use anyhow::Result;
use serde::Serialize;
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

struct Entry<T> {
    value: T,
    expires: Instant,
    used: u64,
    bytes: usize,
}
struct State<T> {
    entries: HashMap<String, Entry<T>>,
    flights: HashMap<String, Weak<tokio::sync::Mutex<()>>>,
    clock: u64,
    epoch: u64,
}
pub(crate) struct ResponseCache<T> {
    state: Mutex<State<T>>,
    capacity: usize,
    max_bytes: usize,
}
impl<T: Clone + Serialize> ResponseCache<T> {
    pub fn new(capacity: usize, max_bytes: usize) -> Self {
        Self {
            state: Mutex::new(State {
                entries: HashMap::new(),
                flights: HashMap::new(),
                clock: 0,
                epoch: 0,
            }),
            capacity,
            max_bytes,
        }
    }
    pub(crate) fn get(&self, key: &str) -> Option<T> {
        let mut state = self.state.lock().unwrap();
        state.clock += 1;
        let clock = state.clock;
        state
            .entries
            .retain(|_, entry| entry.expires > Instant::now());
        state.entries.get_mut(key).map(|entry| {
            entry.used = clock;
            entry.value.clone()
        })
    }
    pub fn invalidate(&self, key: &str) {
        let mut state = self.state.lock().unwrap();
        state.entries.remove(key);
        // An older in-flight result may finish after Refresh. It must not refill the cache.
        state.epoch = state.epoch.wrapping_add(1);
    }
    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap();
        state.entries.clear();
        state.epoch = state.epoch.wrapping_add(1);
    }
    pub async fn load<F, Fut>(&self, key: String, ttl: Duration, fetch: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        self.load_with_ttl(key, |_| ttl, fetch).await
    }
    pub async fn load_with_ttl<F, Fut>(
        &self,
        key: String,
        ttl: impl FnOnce(&T) -> Duration,
        fetch: F,
    ) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        if let Some(value) = self.get(&key) {
            return Ok(value);
        }
        let flight = {
            let mut state = self.state.lock().unwrap();
            state.flights.retain(|_, flight| flight.strong_count() > 0);
            if let Some(flight) = state.flights.get(&key).and_then(Weak::upgrade) {
                flight
            } else {
                let flight = Arc::new(tokio::sync::Mutex::new(()));
                state.flights.insert(key.clone(), Arc::downgrade(&flight));
                flight
            }
        };
        let _flight = flight.lock().await;
        if let Some(value) = self.get(&key) {
            return Ok(value);
        }
        let epoch = self.state.lock().unwrap().epoch;
        let value = fetch().await?;
        let ttl = ttl(&value);
        let bytes = serde_json::to_vec(&value)?.len() + key.len();
        let mut state = self.state.lock().unwrap();
        if state.epoch == epoch && bytes <= self.max_bytes && self.capacity > 0 {
            state.clock += 1;
            let used = state.clock;
            state.entries.insert(
                key,
                Entry {
                    value: value.clone(),
                    expires: Instant::now() + ttl,
                    used,
                    bytes,
                },
            );
            while state.entries.len() > self.capacity
                || state.entries.values().map(|v| v.bytes).sum::<usize>() > self.max_bytes
            {
                let oldest = state
                    .entries
                    .iter()
                    .min_by_key(|(_, v)| v.used)
                    .map(|(k, _)| k.clone())
                    .unwrap();
                state.entries.remove(&oldest);
            }
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn expiration_lru_size_limits_and_errors_allow_fresh_requests() {
        let cache = ResponseCache::new(2, 100);
        for key in ["a", "b"] {
            cache
                .load(key.into(), Duration::from_secs(60), || async {
                    Ok(key.to_owned())
                })
                .await
                .unwrap();
        }
        assert_eq!(cache.get("a").as_deref(), Some("a"));
        cache
            .load("c".into(), Duration::ZERO, || async { Ok("c".into()) })
            .await
            .unwrap();
        assert!(cache.get("b").is_none());
        assert!(cache.get("c").is_none());
        cache
            .load("large".into(), Duration::from_secs(60), || async {
                Ok("x".repeat(101))
            })
            .await
            .unwrap();
        assert!(cache.get("large").is_none());
        let weighted = ResponseCache::new(10, 100);
        for key in ["a", "b"] {
            weighted
                .load(key.into(), Duration::from_secs(60), || async {
                    Ok("x".repeat(60))
                })
                .await
                .unwrap();
        }
        assert!(
            weighted.get("a").is_none(),
            "combined byte weight must evict old entries"
        );
        assert!(weighted.get("b").is_some());
        assert!(
            cache
                .load("error".into(), Duration::from_secs(60), || async {
                    anyhow::bail!("offline")
                })
                .await
                .is_err()
        );
        assert_eq!(
            cache
                .load("error".into(), Duration::from_secs(60), || async {
                    Ok("recovered".into())
                })
                .await
                .unwrap(),
            "recovered"
        );
    }
    #[tokio::test]
    async fn cancellation_and_refresh_do_not_poison_or_restore_old_results() {
        let cache = Arc::new(ResponseCache::new(2, 100));
        let started = Arc::new(tokio::sync::Notify::new());
        let (c, s) = (cache.clone(), started.clone());
        let task = tokio::spawn(async move {
            c.load("key".into(), Duration::from_secs(60), || async {
                s.notify_one();
                std::future::pending::<Result<String>>().await
            })
            .await
        });
        started.notified().await;
        task.abort();
        let _ = task.await;
        let c = cache.clone();
        assert_eq!(
            cache
                .load("key".into(), Duration::from_secs(60), || async {
                    c.invalidate("key");
                    Ok("old".into())
                })
                .await
                .unwrap(),
            "old"
        );
        assert!(cache.get("key").is_none());
        assert_eq!(
            cache
                .load("key".into(), Duration::from_secs(60), || async {
                    Ok("new".into())
                })
                .await
                .unwrap(),
            "new"
        );
        cache.clear();
        assert!(cache.get("key").is_none());
    }
}
