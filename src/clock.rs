use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

/// The time the inbox and store stamp messages with, injectable so tests can move past the 24-hour window.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();

        i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
    }
}

/// Real time shifted by however far a test has moved it, so it still advances between steps.
#[derive(Default)]
pub struct TestClock {
    offset_ms: AtomicI64,
}

impl TestClock {
    pub fn advance(&self, by: Duration) {
        self.offset_ms.fetch_add(
            i64::try_from(by.as_millis()).unwrap_or(i64::MAX),
            Ordering::Relaxed,
        );
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        SystemClock.now_ms() + self.offset_ms.load(Ordering::Relaxed)
    }
}
