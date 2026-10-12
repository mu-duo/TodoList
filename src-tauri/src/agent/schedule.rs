//! 调度：夜窗判定、并发 1 队列、每夜 attempts 计数、退避（**不 sleep 堵队列**）。
//!
//! 仅应用运行期间执行；本地时间每日 `00:00`（含）–`09:00`（不含）可启动新 run，
//! `09:00` 后不再启动（进行中可完成）。每夜 `(task_id, 当地日历夜)` 最多 3 次尝试含首次。
//! 无模型 / 搜索调用总次数帽。

use std::collections::VecDeque;
use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime, Timelike};

use super::record::{RunRecord, RunState};

/// 夜窗结束小时（不含）：09:00。
pub const WINDOW_END_HOUR: u32 = 9;
/// 每夜最多尝试次数（含首次）。
pub const MAX_ATTEMPTS_PER_NIGHT: u32 = 3;
/// 单 run 墙钟上限。
pub const RUN_DEADLINE: Duration = Duration::from_secs(30 * 60);
/// 退避基数。
pub const BACKOFF_BASE: Duration = Duration::from_secs(60);
/// 退避上限。
pub const BACKOFF_MAX: Duration = Duration::from_secs(30 * 60);

/// 是否处于夜窗（`00:00` 含 / `09:00` 不含）。
pub fn in_window(now: NaiveDateTime) -> bool {
    now.hour() < WINDOW_END_HOUR
}

/// 当地日历夜（窗口落在同一自然日内，故即日期）。
pub fn night_date(now: NaiveDateTime) -> NaiveDate {
    now.date()
}

fn parse_local_date(s: &str) -> Option<NaiveDate> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .ok()
        .map(|d| d.date())
}

/// 由运行记录统计 `task_id` 在 `date` 当夜已尝试次数（含手动 force 与首次）。
pub fn attempts_tonight(records: &[RunRecord], task_id: &str, date: NaiveDate) -> u32 {
    records
        .iter()
        .filter(|r| r.task_id == task_id && parse_local_date(&r.started_at) == Some(date))
        .count() as u32
}

/// 退避时长：`base * 2^(attempt-1)`，封顶 [`BACKOFF_MAX`]。
pub fn retry_delay(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(6);
    let factor = 1u32 << shift;
    BACKOFF_BASE
        .checked_mul(factor)
        .map(|d| d.min(BACKOFF_MAX))
        .unwrap_or(BACKOFF_MAX)
}

/// 错误分类（供重试决策）。`Aborted` / `stopped` **不得视为可重试**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Fatal,
    Transient,
    Aborted,
}

/// 由 provider `error_class` 归类。
pub fn classify_error(class: &str) -> ErrorKind {
    match class {
        "Transport" | "HttpStatus" => ErrorKind::Transient,
        "Aborted" => ErrorKind::Aborted,
        // MissingCredential/HttpAuth/Parse/NoResultBlock/Redirect/ProviderError/UnsafePath/… → 致命
        _ => ErrorKind::Fatal,
    }
}

/// 自动调度的准入失败原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    NotEnabled,
    Window,
    AttemptsExhausted,
    AlreadySucceeded,
    ManuallyStopped,
    NotRetryable,
    Running,
}

/// 自动调度准入（手动 `force` 不经过本函数，仅绕窗口但仍计入当夜 attempts）。
///
/// `manual_stopped_tonight` 由持久手动停止标记（本夜）决定：为真则本夜不自动重排。
/// `stopped` 状态**不再**单独长期阻断——手动停止由标记处理，停用后再启用的未成功任务应可重排。
pub fn auto_eligible(
    now: NaiveDateTime,
    enabled: bool,
    attempts: u32,
    manual_stopped_tonight: bool,
    last: Option<&RunRecord>,
) -> Result<(), BlockReason> {
    if !enabled {
        return Err(BlockReason::NotEnabled);
    }
    if !in_window(now) {
        return Err(BlockReason::Window);
    }
    if attempts >= MAX_ATTEMPTS_PER_NIGHT {
        return Err(BlockReason::AttemptsExhausted);
    }
    if manual_stopped_tonight {
        return Err(BlockReason::ManuallyStopped);
    }
    match last {
        None => Ok(()),
        Some(rec) => match RunState::parse(&rec.state) {
            Some(RunState::Running) => Err(BlockReason::Running),
            Some(RunState::Interrupted) => Ok(()),
            Some(RunState::Stopped) => Ok(()),
            Some(RunState::Succeeded) => Err(BlockReason::AlreadySucceeded),
            Some(RunState::Failed) => match rec.error_class.as_deref() {
                Some(class) if classify_error(class) == ErrorKind::Transient => Ok(()),
                _ => Err(BlockReason::NotRetryable),
            },
            _ => Ok(()),
        },
    }
}

/// 队列项（并发 1；`ready_at_ms` 为退避就绪时刻）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub run_id: String,
    pub list_file: String,
    pub task_id: String,
    pub attempt: u32,
    pub ready_at_ms: u64,
    pub force: bool,
}

/// 并发 1 队列。`pop_ready` 跳过未就绪项（不 sleep / 不堵队列）。
#[derive(Debug, Default)]
pub struct Scheduler {
    queue: VecDeque<QueueItem>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    #[allow(dead_code)] // 供测试 / 诊断。
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    #[allow(dead_code)] // 供测试 / 诊断。
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn enqueue(&mut self, item: QueueItem) {
        self.queue.push_back(item);
    }

    /// 移除某任务的排队项（停用 / 完成 / 删除 / 手动停止时取消队列），返回移除数。
    pub fn remove_task(&mut self, list_file: &str, task_id: &str) -> usize {
        let before = self.queue.len();
        self.queue
            .retain(|i| !(i.list_file == list_file && i.task_id == task_id));
        before - self.queue.len()
    }

    pub fn pending(&self) -> Vec<QueueItem> {
        self.queue.iter().cloned().collect()
    }

    /// 弹出第一个 `ready_at_ms <= now_ms` 的项；未就绪项保留在队列中。
    pub fn pop_ready(&mut self, now_ms: u64) -> Option<QueueItem> {
        let pos = self.queue.iter().position(|i| i.ready_at_ms <= now_ms)?;
        self.queue.remove(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::record::{RunSnapshot, RunStats};

    fn dt(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap()
    }

    fn rec(task: &str, state: RunState, class: Option<&str>, started: &str) -> RunRecord {
        RunRecord {
            run_id: ulid::Ulid::generate().to_string(),
            task_id: task.to_string(),
            list_file: "L.yaml".to_string(),
            attempt: 1,
            state: state.as_str().to_string(),
            error_class: class.map(str::to_string),
            stop_reason: None,
            started_at: started.to_string(),
            ended_at: None,
            model: "m".to_string(),
            search_tool: "web_search_20250305".to_string(),
            stats: RunStats::default(),
            snapshot: RunSnapshot {
                title: "t".to_string(),
                notes: String::new(),
                paths: vec![],
            },
            result_dir: "D:\\r".to_string(),
            partial: false,
        }
    }

    #[test]
    fn window_boundaries_00_inclusive_09_exclusive() {
        assert!(in_window(dt(2026, 10, 12, 0, 0)), "00:00 含");
        assert!(in_window(dt(2026, 10, 12, 8, 59)));
        assert!(!in_window(dt(2026, 10, 12, 9, 0)), "09:00 不含");
        assert!(!in_window(dt(2026, 10, 12, 23, 59)));
        assert!(in_window(dt(2026, 10, 12, 3, 0)));
    }

    #[test]
    fn attempts_tonight_counts_and_resets_new_night() {
        let night1 = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let night2 = chrono::NaiveDate::from_ymd_opt(2026, 10, 13).unwrap();
        let records = vec![
            rec("T1", RunState::Failed, Some("Transport"), "2026-10-12T01:00"),
            rec("T1", RunState::Stopped, None, "2026-10-12T05:00"),
            rec("T1", RunState::Succeeded, None, "2026-10-12T07:30"),
            rec("T2", RunState::Failed, Some("Transport"), "2026-10-12T02:00"),
            rec("T1", RunState::Failed, Some("Transport"), "2026-10-13T00:30"),
        ];
        assert_eq!(attempts_tonight(&records, "T1", night1), 3, "含首次 + 手动 stopped");
        assert_eq!(attempts_tonight(&records, "T2", night1), 1);
        assert_eq!(attempts_tonight(&records, "T1", night2), 1, "新夜归零后重新计数");
    }

    #[test]
    fn retry_delay_backs_off_and_caps() {
        assert_eq!(retry_delay(1), BACKOFF_BASE);
        assert_eq!(retry_delay(2), BACKOFF_BASE * 2);
        assert_eq!(retry_delay(9), BACKOFF_MAX);
    }

    #[test]
    fn auto_eligibility_distinguishes_states() {
        let now = dt(2026, 10, 12, 3, 0);
        assert_eq!(
            auto_eligible(now, false, 0, false, None),
            Err(BlockReason::NotEnabled)
        );
        assert_eq!(
            auto_eligible(now, true, 3, false, None),
            Err(BlockReason::AttemptsExhausted)
        );
        assert_eq!(
            auto_eligible(dt(2026, 10, 12, 9, 0), true, 0, false, None),
            Err(BlockReason::Window)
        );
        assert!(auto_eligible(now, true, 0, false, None).is_ok());

        let interrupted = rec("T1", RunState::Interrupted, None, "2026-10-12T01:00");
        assert!(
            auto_eligible(now, true, 1, false, Some(&interrupted)).is_ok(),
            "interrupted 可重试"
        );

        // 手动停止由**本夜标记**处理：标记为真 → 本夜不重排
        let stopped = rec("T1", RunState::Stopped, None, "2026-10-12T01:00");
        assert_eq!(
            auto_eligible(now, true, 1, true, Some(&stopped)),
            Err(BlockReason::ManuallyStopped),
            "本夜手动停止不自动重跑"
        );
        // 标记为假（已进入新夜 / 未手动停止）→ stopped 不再长期阻断（停用后再启用可重排）
        assert!(
            auto_eligible(now, true, 1, false, Some(&stopped)).is_ok(),
            "stopped 不长期阻断：停用后再启用可重排"
        );

        let succeeded = rec("T1", RunState::Succeeded, None, "2026-10-12T01:00");
        assert_eq!(
            auto_eligible(now, true, 1, false, Some(&succeeded)),
            Err(BlockReason::AlreadySucceeded)
        );

        let transient = rec("T1", RunState::Failed, Some("Transport"), "2026-10-12T01:00");
        assert!(
            auto_eligible(now, true, 1, false, Some(&transient)).is_ok(),
            "临时错误可退避重试"
        );
        let fatal = rec("T1", RunState::Failed, Some("MissingCredential"), "2026-10-12T01:00");
        assert_eq!(
            auto_eligible(now, true, 1, false, Some(&fatal)),
            Err(BlockReason::NotRetryable),
            "致命错误不重试"
        );
    }

    #[test]
    fn scheduler_skips_not_ready_without_blocking() {
        let mut sched = Scheduler::new();
        sched.enqueue(QueueItem {
            run_id: "01".into(),
            list_file: "L".into(),
            task_id: "slow".into(),
            attempt: 1,
            ready_at_ms: 1000,
            force: false,
        });
        sched.enqueue(QueueItem {
            run_id: "02".into(),
            list_file: "L".into(),
            task_id: "fast".into(),
            attempt: 1,
            ready_at_ms: 100,
            force: false,
        });

        // now=150：只有 fast 就绪；slow 保留，不阻塞
        let picked = sched.pop_ready(150).unwrap();
        assert_eq!(picked.task_id, "fast");
        assert_eq!(sched.len(), 1);
        assert_eq!(sched.pending()[0].task_id, "slow");

        // now=150：无就绪项 → None（不 sleep）
        assert!(sched.pop_ready(150).is_none());
        // 到达就绪时刻后可取出
        assert_eq!(sched.pop_ready(1000).unwrap().task_id, "slow");
        assert!(sched.is_empty());
    }

    #[test]
    fn scheduler_removes_task_from_queue() {
        let mut sched = Scheduler::new();
        for task in ["A", "B", "A"] {
            sched.enqueue(QueueItem {
                run_id: ulid::Ulid::generate().to_string(),
                list_file: "L".into(),
                task_id: task.into(),
                attempt: 1,
                ready_at_ms: 0,
                force: false,
            });
        }
        assert_eq!(sched.remove_task("L", "A"), 2);
        assert_eq!(sched.len(), 1);
        assert_eq!(sched.pending()[0].task_id, "B");
    }
}
