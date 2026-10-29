//! Public versioned player assets only. No account responses, cookies or stream URLs live here.
use crate::{Error, Result};
use reqwest::blocking::Client;
use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};
use url::Url;
const MAX_ASSET: usize = 4 * 1024 * 1024;
#[derive(Default)]
struct Assets {
    values: HashMap<String, (Instant, Arc<String>)>,
}
impl Assets {
    fn lookup(&mut self, key: &str) -> Option<Arc<String>> {
        self.values
            .retain(|_, (at, _)| at.elapsed() < Duration::from_secs(6 * 3600));
        self.values.get(key).map(|(_, body)| body.clone())
    }
    fn insert(&mut self, key: &str, body: Arc<String>) -> Result<()> {
        if body.len() > MAX_ASSET {
            return Err(Error::UnsupportedYoutube(
                "Player JavaScript exceeds memory budget".into(),
            ));
        }
        while self.values.len() >= 3
            || self.values.values().map(|(_, v)| v.len()).sum::<usize>() + body.len()
                > 8 * 1024 * 1024
        {
            let Some(oldest) = self
                .values
                .iter()
                .min_by_key(|(_, v)| v.0)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.values.remove(&oldest);
        }
        self.values.insert(key.into(), (Instant::now(), body));
        Ok(())
    }
}
#[derive(Default)]
struct SharedAssets {
    assets: Mutex<Assets>,
    flights: Mutex<HashMap<String, Weak<Mutex<()>>>>,
}
impl SharedAssets {
    fn get(&self, key: &str, load: impl FnOnce() -> Result<String>) -> Result<Arc<String>> {
        if let Some(body) = self.assets.lock().expect("player assets").lookup(key) {
            return Ok(body);
        }
        let flight = {
            let mut flights = self.flights.lock().expect("player asset flights");
            flights.retain(|_, v| v.strong_count() > 0);
            match flights.get(key).and_then(Weak::upgrade) {
                Some(flight) => flight,
                None => {
                    let flight = Arc::new(Mutex::new(()));
                    flights.insert(key.into(), Arc::downgrade(&flight));
                    flight
                }
            }
        };
        let _guard = flight.lock().expect("player asset request");
        if let Some(body) = self.assets.lock().expect("player assets").lookup(key) {
            return Ok(body);
        }
        // Independent versions/accounts never hold a global lock during network I/O.
        let body = Arc::new(load()?);
        self.assets
            .lock()
            .expect("player assets")
            .insert(key, body.clone())?;
        Ok(body)
    }
}
pub(super) fn get(client: &Client, url: &Url) -> Result<Arc<String>> {
    let fetch = || {
        let response = super::super::direct::request(client, url, None)?;
        let mut bytes = Vec::new();
        response
            .take(MAX_ASSET as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_ASSET {
            return Err(Error::UnsupportedYoutube(
                "Player JavaScript exceeds memory budget".into(),
            ));
        }
        String::from_utf8(bytes)
            .map_err(|_| Error::UnsupportedYoutube("Invalid player JavaScript text".into()))
    };
    if url.scheme() != "https"
        || !matches!(url.host_str(), Some("www.youtube.com" | "youtube.com"))
        || !url.path().starts_with("/s/player/")
        || !url.query().unwrap_or_default().is_empty()
    {
        return fetch().map(Arc::new);
    }
    static CACHE: OnceLock<SharedAssets> = OnceLock::new();
    CACHE
        .get_or_init(SharedAssets::default)
        .get(url.as_str(), fetch)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versioned_assets_reuse_download_expire_and_stay_bounded() {
        let cache = SharedAssets::default();
        let mut count = 0;
        cache
            .get("player-a", || {
                count += 1;
                Ok("body".into())
            })
            .unwrap();
        cache
            .get("player-a", || {
                count += 1;
                Ok("wrong".into())
            })
            .unwrap();
        assert_eq!(count, 1);
        cache
            .assets
            .lock()
            .unwrap()
            .values
            .get_mut("player-a")
            .unwrap()
            .0 = Instant::now() - Duration::from_secs(7 * 3600);
        assert_eq!(
            &**cache
                .get("player-a", || {
                    count += 1;
                    Ok("new body".into())
                })
                .unwrap(),
            "new body"
        );
        assert_eq!(count, 2);
        assert!(
            cache
                .get("oversize", || Ok("x".repeat(MAX_ASSET + 1)))
                .is_err()
        );
        for n in 0..5 {
            cache.get(&n.to_string(), || Ok("small".into())).unwrap();
        }
        assert!(cache.assets.lock().unwrap().values.len() <= 3);
    }
    #[test]
    fn slow_version_does_not_block_an_independent_version_and_duplicate_fetches_coalesce() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        };
        let cache = Arc::new(SharedAssets::default());
        let (entered, wait_entered) = mpsc::channel();
        let (release, wait_release) = mpsc::channel();
        let slow_cache = cache.clone();
        let slow = std::thread::spawn(move || {
            slow_cache
                .get("slow", || {
                    entered.send(()).unwrap();
                    wait_release.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok("slow result".into())
                })
                .unwrap()
        });
        wait_entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let cache = cache.clone();
                let calls = calls.clone();
                scope.spawn(move || {
                    cache
                        .get("fast", || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok("fast result".into())
                        })
                        .unwrap()
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        release.send(()).unwrap();
        assert_eq!(&**slow.join().unwrap(), "slow result");
    }
}
