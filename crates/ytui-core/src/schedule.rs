//! Daily local-time policy. Start is inclusive, end exclusive, including midnight.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    pub download_window: Option<String>,
    pub bandwidth_window: Option<String>,
    pub window_bandwidth_kib: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    start: u16,
    end: u16,
}
impl Window {
    pub fn parse(value: &str) -> Result<Self> {
        fn minute(value: &str) -> Result<u16> {
            let (h, m) = value
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("Use HH:MM-HH:MM"))?;
            ensure!(h.len() == 2 && m.len() == 2, "Use HH:MM-HH:MM");
            let (h, m) = (h.parse::<u16>()?, m.parse::<u16>()?);
            ensure!(h < 24 && m < 60, "Invalid local time");
            Ok(h * 60 + m)
        }
        let (start, end) = value
            .split_once('-')
            .ok_or_else(|| anyhow::anyhow!("Use HH:MM-HH:MM"))?;
        let window = Self {
            start: minute(start)?,
            end: minute(end)?,
        };
        ensure!(
            window.start != window.end,
            "Window start and end must differ; leave blank for all day"
        );
        Ok(window)
    }
    pub fn contains(self, minute: u16) -> bool {
        if self.start < self.end {
            (self.start..self.end).contains(&minute)
        } else {
            minute >= self.start || minute < self.end
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub allowed: bool,
    pub global_bandwidth_kib: u64,
    pub concurrency: usize,
}
impl Schedule {
    pub fn validate(&self) -> Result<()> {
        for value in [&self.download_window, &self.bandwidth_window]
            .into_iter()
            .flatten()
        {
            Window::parse(value)?;
        }
        Ok(())
    }
    pub fn policy(&self, minute: u16, normal_cap: u64, concurrency: usize) -> Policy {
        let inside = |value: &Option<String>| {
            value
                .as_deref()
                .and_then(|v| Window::parse(v).ok())
                .is_some_and(|w| w.contains(minute))
        };
        Policy {
            allowed: self.download_window.is_none() || inside(&self.download_window),
            global_bandwidth_kib: if inside(&self.bandwidth_window) {
                self.window_bandwidth_kib
            } else {
                normal_cap
            },
            concurrency,
        }
    }
}
/// Zero means unlimited; combining limits must never remove a nonzero cap.
pub fn cap(a: u64, b: u64) -> u64 {
    match (a, b) {
        (0, b) => b,
        (a, 0) => a,
        (a, b) => a.min(b),
    }
}
pub fn rate_bytes(per_job_kib: u64, global_kib: u64, concurrency: usize) -> u64 {
    let share = if global_kib == 0 {
        0
    } else {
        (global_kib.saturating_mul(1024) / concurrency.max(1) as u64).max(1)
    };
    cap(per_job_kib.saturating_mul(1024), share)
}
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn delay_seconds(value: &str) -> Result<u64> {
    let (number, multiplier) = if let Some(v) = value.strip_suffix('m') {
        (v, 60)
    } else if let Some(v) = value.strip_suffix('h') {
        (v, 3600)
    } else {
        (value.strip_suffix('s').unwrap_or(value), 1)
    };
    let seconds = number
        .parse::<u64>()?
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("Delay is too large"))?;
    ensure!(
        (1..=366 * 86400).contains(&seconds),
        "Delay must be between 1 second and 366 days"
    );
    Ok(seconds)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn midnight_windows_and_limits_have_exact_boundaries() {
        let schedule = Schedule {
            download_window: Some("22:30-06:00".into()),
            bandwidth_window: Some("23:00-05:00".into()),
            window_bandwidth_kib: 1,
        };
        schedule.validate().unwrap();
        assert!(!schedule.policy(22 * 60 + 29, 0, 16).allowed);
        assert!(schedule.policy(22 * 60 + 30, 0, 16).allowed);
        assert!(schedule.policy(0, 0, 16).allowed);
        assert!(!schedule.policy(6 * 60, 0, 16).allowed);
        assert_eq!(schedule.policy(0, 0, 16).global_bandwidth_kib, 1);
        assert_eq!(rate_bytes(0, 1, 16), 64); // Never rounds a small KiB cap to unlimited.
        assert_eq!(rate_bytes(1, 32, 16), 1024);
        for v in ["24:00-06:00", "00:00-00:00", "1:00-02:00", "no"] {
            assert!(Window::parse(v).is_err());
        }
        assert_eq!(delay_seconds("2h").unwrap(), 7200);
        assert!(delay_seconds("0m").is_err());
    }
}
