//! 注入式时钟：夜窗判定用本地时间，run deadline 用单调毫秒。
//!
//! 生产用 [`SystemClock`]；测试用 [`FakeClock`]（`#[cfg]` 门禁）驱动窗口与 timeout。

use std::time::Instant;

use chrono::NaiveDateTime;

/// 时钟抽象。
pub trait Clock: Send + Sync {
    /// 当前本地时间（夜窗 / 当地日历夜 / 记录时间戳）。
    fn now_local(&self) -> NaiveDateTime;
    /// 单调毫秒（run 墙钟 deadline；不随系统时钟回拨跳变）。
    fn monotonic_ms(&self) -> u64;
}

/// 系统时钟。
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_local(&self) -> NaiveDateTime {
        chrono::Local::now().naive_local()
    }
    fn monotonic_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

/// 测试用可控时钟。
#[cfg(any(test, feature = "agent-test-provider"))]
pub struct FakeClock {
    inner: std::sync::Mutex<FakeState>,
}

#[cfg(any(test, feature = "agent-test-provider"))]
struct FakeState {
    local: NaiveDateTime,
    mono: u64,
    auto_step_ms: u64,
}

#[cfg(any(test, feature = "agent-test-provider"))]
#[allow(dead_code)] // feature 构建下部分驱动方法仅供测试。
impl FakeClock {
    pub fn new(local: NaiveDateTime) -> Self {
        Self {
            inner: std::sync::Mutex::new(FakeState {
                local,
                mono: 0,
                auto_step_ms: 0,
            }),
        }
    }

    #[allow(dead_code)] // 供测试驱动窗口。
    pub fn set_local(&self, local: NaiveDateTime) {
        self.inner.lock().unwrap().local = local;
    }

    pub fn advance_local_secs(&self, secs: i64) {
        let mut s = self.inner.lock().unwrap();
        s.local += chrono::Duration::seconds(secs);
    }

    /// 每次读取 `monotonic_ms` 后自动前进 `step` 毫秒（驱动 deadline 测试）。
    pub fn set_auto_step_ms(&self, step: u64) {
        self.inner.lock().unwrap().auto_step_ms = step;
    }

    #[allow(dead_code)] // 供测试驱动 deadline。
    pub fn advance_mono(&self, ms: u64) {
        self.inner.lock().unwrap().mono += ms;
    }
}

#[cfg(any(test, feature = "agent-test-provider"))]
impl Clock for FakeClock {
    fn now_local(&self) -> NaiveDateTime {
        self.inner.lock().unwrap().local
    }
    fn monotonic_ms(&self) -> u64 {
        let mut s = self.inner.lock().unwrap();
        let now = s.mono;
        s.mono += s.auto_step_ms;
        now
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Timelike};

    #[test]
    fn fake_clock_advances_and_steps() {
        let base = NaiveDate::from_ymd_opt(2026, 10, 12)
            .unwrap()
            .and_hms_opt(3, 0, 0)
            .unwrap();
        let clock = FakeClock::new(base);
        assert_eq!(clock.now_local(), base);
        clock.advance_local_secs(3600);
        assert_eq!(clock.now_local().hour(), 4);

        clock.set_auto_step_ms(1000);
        assert_eq!(clock.monotonic_ms(), 0);
        assert_eq!(clock.monotonic_ms(), 1000);
        assert_eq!(clock.monotonic_ms(), 2000);
    }
}
