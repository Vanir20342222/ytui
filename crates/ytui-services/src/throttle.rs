//! Media-byte pacing runs only inside the isolated native worker.
use std::time::{Duration, Instant};
use yt_dlp_rs::{DownloadPhase, DownloadProgress};
pub struct Throttle {
    rate: u64,
    started: Instant,
    bytes: u64,
    last: u64,
    stream: Option<u8>,
}
impl Throttle {
    pub fn new(rate: u64) -> Self {
        Self {
            rate,
            started: Instant::now(),
            bytes: 0,
            last: 0,
            stream: None,
        }
    }
    pub fn new_resource(&mut self) {
        self.last = 0;
        self.stream = None;
    }
    fn observe(&mut self, progress: DownloadProgress, elapsed: Duration) -> Duration {
        if progress.phase != DownloadPhase::Downloading || self.rate == 0 {
            return Duration::ZERO;
        }
        if self.stream != progress.stream_index {
            self.last = 0;
            self.stream = progress.stream_index;
        }
        let delta = if progress.downloaded_bytes >= self.last {
            progress.downloaded_bytes - self.last
        } else {
            progress.downloaded_bytes
        };
        self.bytes = self.bytes.saturating_add(delta);
        self.last = progress.downloaded_bytes;
        Duration::try_from_secs_f64(self.bytes as f64 / self.rate as f64)
            .unwrap_or(Duration::MAX)
            .saturating_sub(elapsed)
    }
    pub fn pace(&mut self, progress: DownloadProgress) {
        let delay = self.observe(progress, self.started.elapsed());
        // SIGTERM from the supervisor interrupts this worker, including a low-rate wait.
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn progress(bytes: u64, stream: Option<u8>, fragment: Option<u32>) -> DownloadProgress {
        DownloadProgress {
            attempt: 1,
            downloaded_bytes: bytes,
            total_bytes: None,
            phase: DownloadPhase::Downloading,
            stream_index: stream,
            stream_count: None,
            fragment_index: fragment,
            fragment_count: None,
        }
    }
    #[test]
    fn counts_separate_streams_and_cumulative_fragments_without_losing_the_cap() {
        let mut throttle = Throttle::new(1024);
        assert_eq!(
            throttle.observe(progress(1024, Some(1), None), Duration::ZERO),
            Duration::from_secs(1)
        );
        assert_eq!(
            throttle.observe(progress(1024, Some(2), None), Duration::from_secs(1)),
            Duration::from_secs(1)
        );
        throttle.new_resource();
        assert_eq!(
            throttle.observe(progress(512, None, Some(1)), Duration::from_secs(2)),
            Duration::from_millis(500)
        );
        assert_eq!(
            throttle.observe(progress(1024, None, Some(2)), Duration::from_secs(2)),
            Duration::from_secs(1)
        );
        assert_eq!(
            throttle.observe(progress(1024, None, Some(2)), Duration::from_secs(3)),
            Duration::ZERO
        );
        let mut unlimited = Throttle::new(0);
        assert_eq!(
            unlimited.observe(progress(u64::MAX, None, None), Duration::ZERO),
            Duration::ZERO
        );
    }
}
