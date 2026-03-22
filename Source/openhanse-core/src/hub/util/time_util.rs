use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct TimeUtil;

impl TimeUtil {
    pub fn unix_time_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be after unix epoch")
            .as_millis()
            .min(u128::from(u64::MAX)) as u64
    }

    pub fn unix_time_ms_from_now(delta: Duration) -> u64 {
        Self::unix_time_ms().saturating_add(delta.as_millis().min(u128::from(u64::MAX)) as u64)
    }

    pub fn unix_time_ms_from_instant(then: Instant, now: Instant) -> u64 {
        let age = now.saturating_duration_since(then);
        Self::unix_time_ms().saturating_sub(age.as_millis().min(u128::from(u64::MAX)) as u64)
    }
}
