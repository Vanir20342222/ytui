//! Session-only timer. Delivery of Stop is retried by the app under backpressure.
use anyhow::{Result, ensure};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct SleepTimer {
    deadline: Option<Instant>,
    stopping: bool,
    shown_remaining: Option<u64>,
}
impl SleepTimer {
    pub fn arm(&mut self, duration: Duration, now: Instant) -> Result<()> {
        ensure!(
            (Duration::from_secs(1)..=Duration::from_secs(86400)).contains(&duration),
            "Sleep duration must be 1 second to 24 hours"
        );
        let deadline = now
            .checked_add(duration)
            .ok_or_else(|| anyhow::anyhow!("Sleep deadline is out of range"))?;
        self.deadline = Some(deadline);
        self.stopping = false;
        Ok(())
    }
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
    pub fn due(&mut self, now: Instant) -> bool {
        self.stopping |= self.deadline.is_some_and(|at| now >= at);
        self.stopping
    }
    pub fn needs_redraw(&mut self, now: Instant) -> bool {
        let remaining = self.remaining(now);
        let changed = remaining != self.shown_remaining;
        self.shown_remaining = remaining;
        changed
    }
    pub fn stopping(&self) -> bool {
        self.stopping
    }
    pub fn remaining(&self, now: Instant) -> Option<u64> {
        self.deadline.map(|at| {
            let remaining = at.saturating_duration_since(now);
            remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn countdown_rounds_up_and_expires_exactly_at_the_deadline() {
        let now = Instant::now();
        let mut timer = SleepTimer::default();
        timer.arm(Duration::from_secs(60), now).unwrap();
        assert_eq!(timer.remaining(now + Duration::from_millis(1)), Some(60));
        assert!(!timer.due(now + Duration::from_secs(59)));
        assert!(timer.due(now + Duration::from_secs(60)));
        assert!(timer.stopping());
        assert!(timer.due(now + Duration::from_secs(61)));
    }
    #[test]
    fn invalid_changes_leave_a_running_timer_intact() {
        let now = Instant::now();
        let mut timer = SleepTimer::default();
        timer.arm(Duration::from_secs(30), now).unwrap();
        for invalid in [Duration::ZERO, Duration::from_secs(86401), Duration::MAX] {
            assert!(timer.arm(invalid, now).is_err());
            assert_eq!(timer.remaining(now), Some(30));
        }
    }
    #[test]
    fn cancel_and_rearm_clear_expired_stop_requests() {
        let now = Instant::now();
        let mut timer = SleepTimer::default();
        timer.arm(Duration::from_secs(1), now).unwrap();
        assert!(timer.due(now + Duration::from_secs(1)));
        timer.cancel();
        assert!(!timer.due(now + Duration::from_secs(2)));
        assert_eq!(timer.remaining(now), None);
        timer.arm(Duration::from_secs(10), now).unwrap();
        assert!(!timer.stopping());
    }
}

#[cfg(test)]
mod redraw_tests {
    use super::*;
    #[test]
    fn idle_countdown_requests_only_one_redraw_per_changed_second() {
        let now = Instant::now();
        let mut timer = SleepTimer::default();
        timer.arm(Duration::from_secs(10), now).unwrap();
        assert!(timer.needs_redraw(now));
        assert!(!timer.needs_redraw(now + Duration::from_millis(100)));
        assert!(timer.needs_redraw(now + Duration::from_secs(1)));
        assert!(!timer.needs_redraw(now + Duration::from_millis(1100)));
    }
}
