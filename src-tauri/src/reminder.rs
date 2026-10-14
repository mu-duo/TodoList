//! Pure reminder scheduling primitives. This module deliberately does not own a
//! timer, perform I/O, or invoke an operating-system notification API.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// A task snapshot sufficient to make one deterministic reminder plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderTask {
    pub list_file: String,
    pub task_id: String,
    pub title: String,
    pub status: String,
    pub due: Option<String>,
    /// 任务级提醒开关；缺省 / `reminder: true` 归一化为 `true`。
    pub reminder_enabled: bool,
}

/// 一个任务默认的四个提醒档位；旧版 `advance` 仅兼容读取，绝不再生成。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderEventType {
    Advance7,
    Advance1,
    Advance1h,
    Due,
    /// 旧版 1.5.0 账本中的 `advance`：仅用于读取兼容，调度不会再产生它。
    Advance,
}

/// Stable identity independent of a list's current file name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ReminderEventKey {
    pub task_id: String,
    pub canonical_due: String,
    pub event_type: ReminderEventType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderEvent {
    pub key: ReminderEventKey,
    /// 归属清单文件名；写入通知 `extra` 供「不再提醒」动作回写定位。
    pub list_file: String,
    pub task_title: String,
    pub scheduled_local: NaiveDateTime,
    pub scheduled_at: DateTime<Utc>,
}

/// Serde-friendly ledger state. Persisting this value is the adapter's
/// responsibility; `claim` only returns an updated in-memory copy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderLedger {
    /// 完整快照中唯一 task ID 的最近一次是否 open（`true=open`、`false=非 open`）。
    ///
    /// 用于跨重启识别 `done→open` 重新开启：抑制当前全部逾期未试事件。
    /// 旧文件可能仍带该字段，需读取并沿用（升级连续性）。
    #[serde(default)]
    pub last_status: BTreeMap<String, bool>,
    /// 完整快照中 open + 开启 + due 合法任务的 canonical due 标记。
    ///
    /// 缺失或与当前 canonical due 不同 → 首帧：静默记账已过的提前档。
    /// 旧账本无该字段也可读。
    #[serde(default)]
    pub observed: BTreeMap<String, String>,
    #[serde(default)]
    pub attempted: BTreeSet<ReminderEventKey>,
}

impl ReminderLedger {
    pub fn claim(&mut self, keys: impl IntoIterator<Item = ReminderEventKey>) {
        self.attempted.extend(keys);
    }

    pub fn claim_plan(&mut self, plan: &ReminderPlan) {
        self.claim(plan.claimed_ledger_keys.iter().cloned());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReminderDiagnostic {
    DuplicateTaskIds(Vec<String>),
    InvalidDue {
        task_id: String,
        due: String,
    },
    AdvanceDateOutOfRange {
        task_id: String,
        due: String,
    },
    LocalTimeUnresolvable {
        task_id: String,
        event_type: ReminderEventType,
        requested_local: NaiveDateTime,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyReminder {
    pub event: ReminderEvent,
    /// This task's overdue, previously unclaimed keys; future events are excluded.
    pub claim_keys: Vec<ReminderEventKey>,
    /// 合并补发文案覆盖：`Some` → 使用真实剩余时长文案；`None` → 调用方按档位文案。
    pub body: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReminderPlan {
    /// At most one catch-up notification per open task in this reconciliation.
    pub ready_notifications: Vec<ReadyReminder>,
    /// Unattempted events whose resolved instant is still in the future.
    pub future_events: Vec<ReminderEvent>,
    pub next_due: Option<DateTime<Utc>>,
    /// All overdue, previously-unclaimed events covered by this reconciliation.
    pub claimed_ledger_keys: Vec<ReminderEventKey>,
    pub duplicate_task_ids: Vec<String>,
    pub diagnostics: Vec<ReminderDiagnostic>,
}

/// 一次「完整快照」观察的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservationOutcome {
    /// 首帧静默记账（写入 `attempted`）的已过提前档 keys。
    pub claimed_ledger_keys: Vec<ReminderEventKey>,
    pub diagnostics: Vec<ReminderDiagnostic>,
    /// ledger 是否被改动（供 adapter 决定是否落盘）。
    pub changed: bool,
}

/// 单个接近其计划时刻的档位视为「按时」的容差；沿用运行时最大轮询间隔（60s）。
///
/// 用途：区分「按时触发」与「迟到补发」。运行时的轮询会在 `next_due` 醒来，正常调度
/// 抖动 ≤ 最大轮询间隔，故此刻单一逾期档位按档位文案（还有 N 天/小时到期）；真正迟到
/// 的唤醒（系统睡眠 / 挂起 / 长时间阻塞等，间隔远超 60s）走真实剩余时长文案。
const ON_TIME_DRIFT: Duration = Duration::seconds(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalResolution {
    Single(DateTime<Utc>),
    None,
    Ambiguous(DateTime<Utc>, DateTime<Utc>),
}

/// Resolves a wall-clock time in the machine's local time zone.
pub trait LocalTimeResolver {
    fn resolve_local(&self, local: NaiveDateTime) -> LocalResolution;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemLocalTimeResolver;

impl LocalTimeResolver for SystemLocalTimeResolver {
    fn resolve_local(&self, local: NaiveDateTime) -> LocalResolution {
        match Local.from_local_datetime(&local) {
            LocalResult::Single(value) => LocalResolution::Single(value.with_timezone(&Utc)),
            LocalResult::None => LocalResolution::None,
            LocalResult::Ambiguous(first, second) => {
                LocalResolution::Ambiguous(first.with_timezone(&Utc), second.with_timezone(&Utc))
            }
        }
    }
}

/// Construct a schedule from a caller-provided immutable task snapshot and
/// ledger. `now` is an absolute UTC instant; all due values remain local wall
/// times and are resolved through `resolver`.
pub fn reconcile<R: LocalTimeResolver>(
    tasks: &[ReminderTask],
    ledger: &ReminderLedger,
    now: DateTime<Utc>,
    resolver: &R,
) -> ReminderPlan {
    let mut plan = ReminderPlan::default();
    let mut task_ids: HashMap<&str, usize> = HashMap::new();
    for task in tasks {
        *task_ids.entry(&task.task_id).or_default() += 1;
    }
    plan.duplicate_task_ids = task_ids
        .into_iter()
        .filter_map(|(task_id, count)| (count > 1).then_some(task_id.to_string()))
        .collect();
    plan.duplicate_task_ids.sort();
    if !plan.duplicate_task_ids.is_empty() {
        plan.diagnostics.push(ReminderDiagnostic::DuplicateTaskIds(
            plan.duplicate_task_ids.clone(),
        ));
    }
    let duplicates: HashSet<String> = plan.duplicate_task_ids.iter().cloned().collect();

    for task in tasks {
        if task.status != "open" || duplicates.contains(&task.task_id) {
            continue;
        }
        if !task.reminder_enabled {
            continue;
        }
        let Some(due_value) = task.due.as_deref() else {
            continue;
        };
        let Some((canonical_due, due_local)) = parse_due(due_value) else {
            plan.diagnostics.push(ReminderDiagnostic::InvalidDue {
                task_id: task.task_id.clone(),
                due: due_value.to_string(),
            });
            continue;
        };

        let (events, spec_diagnostics) = event_specs(&task.task_id, &canonical_due, due_local);
        plan.diagnostics.extend(spec_diagnostics);
        append_events(
            task,
            &canonical_due,
            events,
            ledger,
            now,
            resolver,
            &mut plan,
        );
    }

    plan.ready_notifications.sort_by(|a, b| {
        a.event
            .scheduled_at
            .cmp(&b.event.scheduled_at)
            .then(a.event.key.cmp(&b.event.key))
    });
    plan.future_events
        .sort_by(|a, b| a.scheduled_at.cmp(&b.scheduled_at).then(a.key.cmp(&b.key)));
    plan.claimed_ledger_keys.sort();
    plan.next_due = plan
        .future_events
        .iter()
        .map(|event| event.scheduled_at)
        .min();
    plan
}

/// 观察一次「完整快照」：更新 `last_status` / `observed` 标记，并静默记账首帧与重开任务
/// 已过的事件。
///
/// 重开（`last_status == false` 且当前 open）→ claim 该任务当前全部逾期未试事件
/// （advance7/advance1/advance1h 与 due 都算），不发送，未来事件照常排程。
///
/// 首帧 = `observed` 缺少该任务或 canonical due 变化（新建 / 改 due / 升级 / 重新开启）。
/// 首帧只 claim 已过的提前档（advance7/advance1/advance1h），保留已过的「到期档」进入
/// 正常 catch-up —— 重开路径已先 claim 到期档，故重开不会补发。
///
/// `last_status` 与 `observed` 都只在完整快照更新；重复 ID 跳过更新保留旧值，
/// 缺席 ID 予以清理。调用方保证只在完整快照时调用（部分快照不得触碰这些标记）。
pub fn observe_snapshot<R: LocalTimeResolver>(
    tasks: &[ReminderTask],
    ledger: &mut ReminderLedger,
    now: DateTime<Utc>,
    resolver: &R,
) -> ObservationOutcome {
    let mut outcome = ObservationOutcome::default();

    let mut counts = HashMap::<&str, usize>::new();
    for task in tasks {
        *counts.entry(&task.task_id).or_default() += 1;
    }
    let present: HashSet<&str> = counts.keys().copied().collect();

    for task in tasks {
        if counts.get(task.task_id.as_str()) != Some(&1) {
            // 重复 ID 有歧义：保留上次 last_status / observed 标记，不记账。
            continue;
        }

        let is_open = task.status == "open";

        // 1) 重开抑制：上次观察到非 open、本次 open → 静默 claim 全部已过事件。
        if is_open && ledger.last_status.get(&task.task_id) == Some(&false) {
            if let Some((canonical_due, due_local)) = valid_open_due(task) {
                let keys = overdue_event_keys(
                    task,
                    &canonical_due,
                    due_local,
                    now,
                    resolver,
                    &mut outcome.diagnostics,
                );
                if !keys.is_empty() {
                    ledger.attempted.extend(keys.iter().cloned());
                    outcome.claimed_ledger_keys.extend(keys);
                    outcome.changed = true;
                }
            }
        }

        // 2) 记录最新 open 状态。
        if ledger.last_status.get(&task.task_id) != Some(&is_open) {
            ledger.last_status.insert(task.task_id.clone(), is_open);
            outcome.changed = true;
        }

        // 3) 首帧播种（不变）：open + 开启 + due 合法才建立 observed。
        let Some((canonical_due, due_local)) = valid_open_due(task) else {
            if ledger.observed.remove(&task.task_id).is_some() {
                outcome.changed = true;
            }
            continue;
        };

        if ledger.observed.get(&task.task_id) != Some(&canonical_due) {
            let keys = first_frame_advance_keys(
                task,
                &canonical_due,
                due_local,
                ledger,
                now,
                resolver,
                &mut outcome.diagnostics,
            );
            if !keys.is_empty() {
                ledger.attempted.extend(keys.iter().cloned());
                outcome.claimed_ledger_keys.extend(keys);
            }
            ledger.observed.insert(task.task_id.clone(), canonical_due);
            outcome.changed = true;
        }
    }

    let observed_before = ledger.observed.len();
    ledger.observed.retain(|task_id, _| present.contains(task_id.as_str()));
    if ledger.observed.len() != observed_before {
        outcome.changed = true;
    }
    let status_before = ledger.last_status.len();
    ledger.last_status.retain(|task_id, _| present.contains(task_id.as_str()));
    if ledger.last_status.len() != status_before {
        outcome.changed = true;
    }

    outcome
}

/// open + 开启提醒 + due 合法时返回 `(canonical_due, due_local)`，否则 `None`。
fn valid_open_due(task: &ReminderTask) -> Option<(String, NaiveDateTime)> {
    if task.status != "open" || !task.reminder_enabled {
        return None;
    }
    let due = task.due.as_deref()?;
    parse_due(due)
}

fn first_frame_advance_keys<R: LocalTimeResolver>(
    task: &ReminderTask,
    canonical_due: &str,
    due_local: NaiveDateTime,
    ledger: &ReminderLedger,
    now: DateTime<Utc>,
    resolver: &R,
    diagnostics: &mut Vec<ReminderDiagnostic>,
) -> Vec<ReminderEventKey> {
    overdue_event_keys(task, canonical_due, due_local, now, resolver, diagnostics)
        .into_iter()
        // 首帧保留已过的「到期档」：交由正常 catch-up 补发。
        .filter(|key| {
            key.event_type != ReminderEventType::Due && !ledger.attempted.contains(key)
        })
        .collect()
}

/// 该任务当前全部「已过」（`scheduled_at <= now`）的档位 keys（含 due）。
fn overdue_event_keys<R: LocalTimeResolver>(
    task: &ReminderTask,
    canonical_due: &str,
    due_local: NaiveDateTime,
    now: DateTime<Utc>,
    resolver: &R,
    diagnostics: &mut Vec<ReminderDiagnostic>,
) -> Vec<ReminderEventKey> {
    let (specs, spec_diagnostics) = event_specs(&task.task_id, canonical_due, due_local);
    diagnostics.extend(spec_diagnostics);

    let mut keys = Vec::new();
    for (event_type, requested_local) in specs {
        let Some(event) = make_event(task, canonical_due, event_type, requested_local, resolver)
        else {
            diagnostics.push(ReminderDiagnostic::LocalTimeUnresolvable {
                task_id: task.task_id.clone(),
                event_type,
                requested_local,
            });
            continue;
        };
        if event.scheduled_at <= now {
            keys.push(event.key);
        }
    }
    keys
}

/// 生成一个任务的全部档位（顺序：advance7 / advance1 / advance1h / due）。
fn event_specs(
    task_id: &str,
    canonical_due: &str,
    due_local: NaiveDateTime,
) -> (Vec<(ReminderEventType, NaiveDateTime)>, Vec<ReminderDiagnostic>) {
    let mut events = Vec::with_capacity(4);
    let mut diagnostics = Vec::new();
    for (days, event_type) in [
        (7i64, ReminderEventType::Advance7),
        (1i64, ReminderEventType::Advance1),
    ] {
        match advance_local_days(task_id, canonical_due, due_local, days) {
            Ok(local) => events.push((event_type, local)),
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    match due_local.checked_sub_signed(Duration::hours(1)) {
        Some(local) => events.push((ReminderEventType::Advance1h, local)),
        None => diagnostics.push(ReminderDiagnostic::AdvanceDateOutOfRange {
            task_id: task_id.to_string(),
            due: canonical_due.to_string(),
        }),
    }
    events.push((ReminderEventType::Due, due_local));
    (events, diagnostics)
}

fn append_events<R: LocalTimeResolver>(
    task: &ReminderTask,
    canonical_due: &str,
    specs: Vec<(ReminderEventType, NaiveDateTime)>,
    ledger: &ReminderLedger,
    now: DateTime<Utc>,
    resolver: &R,
    plan: &mut ReminderPlan,
) {
    let mut overdue = Vec::new();
    let mut due_instant: Option<DateTime<Utc>> = None;
    for (event_type, requested_local) in specs {
        let Some(event) = make_event(task, canonical_due, event_type, requested_local, resolver)
        else {
            plan.diagnostics
                .push(ReminderDiagnostic::LocalTimeUnresolvable {
                    task_id: task.task_id.clone(),
                    event_type,
                    requested_local,
                });
            continue;
        };
        if event_type == ReminderEventType::Due {
            due_instant = Some(event.scheduled_at);
        }
        if ledger.attempted.contains(&event.key) {
            continue;
        }
        if event.scheduled_at <= now {
            overdue.push(event);
        } else {
            plan.future_events.push(event);
        }
    }
    if !overdue.is_empty() {
        overdue.sort_by(|a, b| a.scheduled_at.cmp(&b.scheduled_at).then(a.key.cmp(&b.key)));
        let body = catch_up_body(due_instant, &overdue, now);
        let claim_keys: Vec<_> = overdue.iter().map(|event| event.key.clone()).collect();
        let representative = overdue
            .into_iter()
            .last()
            .expect("overdue is non-empty");
        plan.ready_notifications.push(ReadyReminder {
            event: representative,
            claim_keys: claim_keys.clone(),
            body,
        });
        plan.claimed_ledger_keys.extend(claim_keys);
    }
}

/// 合并补发的文案覆盖：
/// - 已过 due → `已到截止时间`；
/// - 单个且接近计划时刻的档位 → `None`（交给调用方按档位文案）；
/// - 否则按真实剩余时长：`≥24h` → `还有 N 天到期`（floor(h/24)，最小 1），
///   `<24h` → `还有 N 小时到期`（ceil(min/60)，最小 1）。
fn catch_up_body(
    due_instant: Option<DateTime<Utc>>,
    overdue: &[ReminderEvent],
    now: DateTime<Utc>,
) -> Option<String> {
    let due = due_instant?;
    if due <= now {
        return Some("已到截止时间".to_string());
    }
    if overdue.len() == 1 && (now - overdue[0].scheduled_at) <= ON_TIME_DRIFT {
        // 单一档位且落在正常调度抖动内 → 按档位文案（交给调用方），
        // 避免把「按时」的 T-7d 误算成真实的 6 天多。
        return None;
    }
    Some(remaining_body(due - now))
}

fn remaining_body(remaining: Duration) -> String {
    if remaining >= Duration::hours(24) {
        let days = (remaining.num_hours() / 24).max(1);
        format!("还有 {days} 天到期")
    } else {
        let hours = ((remaining.num_minutes() + 59) / 60).max(1);
        format!("还有 {hours} 小时到期")
    }
}

fn advance_local_days(
    task_id: &str,
    canonical_due: &str,
    local: NaiveDateTime,
    days: i64,
) -> Result<NaiveDateTime, ReminderDiagnostic> {
    local
        .date()
        .checked_sub_signed(Duration::days(days))
        .map(|date| date.and_time(local.time()))
        .ok_or_else(|| ReminderDiagnostic::AdvanceDateOutOfRange {
            task_id: task_id.to_string(),
            due: canonical_due.to_string(),
        })
}

fn make_event<R: LocalTimeResolver>(
    task: &ReminderTask,
    canonical_due: &str,
    event_type: ReminderEventType,
    scheduled_local: NaiveDateTime,
    resolver: &R,
) -> Option<ReminderEvent> {
    let (scheduled_local, scheduled_at) = resolve_wall_time(scheduled_local, resolver)?;
    Some(ReminderEvent {
        key: ReminderEventKey {
            task_id: task.task_id.clone(),
            canonical_due: canonical_due.to_string(),
            event_type,
        },
        list_file: task.list_file.clone(),
        task_title: task.title.clone(),
        scheduled_local,
        scheduled_at,
    })
}

const MAX_LOCAL_RESOLUTION_MINUTES: usize = 48 * 60;

fn resolve_wall_time<R: LocalTimeResolver>(
    requested: NaiveDateTime,
    resolver: &R,
) -> Option<(NaiveDateTime, DateTime<Utc>)> {
    let mut local = requested;
    for _ in 0..=MAX_LOCAL_RESOLUTION_MINUTES {
        match resolver.resolve_local(local) {
            LocalResolution::Single(value) => return Some((local, value)),
            LocalResolution::Ambiguous(first, second) => return Some((local, first.min(second))),
            LocalResolution::None => {
                local = local.checked_add_signed(Duration::minutes(1))?;
            }
        }
    }
    None
}

fn parse_due(value: &str) -> Option<(String, NaiveDateTime)> {
    if value.len() == 10 {
        let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
        if date.format("%Y-%m-%d").to_string() != value {
            return None;
        }
        return Some((value.to_string(), date.and_hms_opt(9, 0, 0)?));
    }
    if value.len() == 16 {
        let datetime = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").ok()?;
        if datetime.format("%Y-%m-%dT%H:%M").to_string() != value {
            return None;
        }
        return Some((value.to_string(), datetime));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;
    use std::cell::Cell;

    #[derive(Default)]
    struct FakeResolver {
        answers: HashMap<NaiveDateTime, LocalResolution>,
    }

    impl FakeResolver {
        fn map(&mut self, local: NaiveDateTime, resolution: LocalResolution) {
            self.answers.insert(local, resolution);
        }
    }

    impl LocalTimeResolver for FakeResolver {
        fn resolve_local(&self, local: NaiveDateTime) -> LocalResolution {
            self.answers
                .get(&local)
                .cloned()
                .unwrap_or_else(|| LocalResolution::Single(local.and_utc()))
        }
    }

    struct NeverResolver {
        calls: Cell<usize>,
    }

    impl LocalTimeResolver for NeverResolver {
        fn resolve_local(&self, _local: NaiveDateTime) -> LocalResolution {
            self.calls.set(self.calls.get() + 1);
            LocalResolution::None
        }
    }

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    fn utc(value: NaiveDateTime) -> DateTime<Utc> {
        value.and_utc()
    }

    fn task(id: &str, due: &str) -> ReminderTask {
        ReminderTask {
            list_file: format!("{id}.yaml"),
            task_id: id.to_string(),
            title: format!("title {id}"),
            status: "open".to_string(),
            due: Some(due.to_string()),
            reminder_enabled: true,
        }
    }

    fn disabled(mut task: ReminderTask) -> ReminderTask {
        task.reminder_enabled = false;
        task
    }

    fn plan(task: &ReminderTask, now: NaiveDateTime) -> ReminderPlan {
        reconcile(
            std::slice::from_ref(task),
            &ReminderLedger::default(),
            utc(now),
            &FakeResolver::default(),
        )
    }

    fn event(plan: &ReminderPlan, event_type: ReminderEventType) -> &ReminderEvent {
        plan.future_events
            .iter()
            .find(|event| event.key.event_type == event_type)
            .unwrap_or_else(|| panic!("missing future event {event_type:?}"))
    }

    #[test]
    fn date_due_is_nine_and_datetime_due_uses_exact_wall_time() {
        let date = task("date", "2026-10-13");
        let date_plan = plan(&date, dt(2026, 9, 1, 0, 0));
        assert_eq!(
            event(&date_plan, ReminderEventType::Due).scheduled_local,
            dt(2026, 10, 13, 9, 0)
        );
        assert_eq!(
            event(&date_plan, ReminderEventType::Due).key.canonical_due,
            "2026-10-13"
        );

        let timed = task("timed", "2026-10-13T14:35");
        let timed_plan = plan(&timed, dt(2026, 9, 1, 0, 0));
        assert_eq!(
            event(&timed_plan, ReminderEventType::Due).scheduled_local,
            dt(2026, 10, 13, 14, 35)
        );
    }

    #[test]
    fn four_events_generated_for_date_only_and_datetime_due() {
        let cases = [
            // (due, advance7, advance1, advance1h, due_local)
            (
                "2026-10-13",
                dt(2026, 10, 6, 9, 0),
                dt(2026, 10, 12, 9, 0),
                dt(2026, 10, 13, 8, 0),
                dt(2026, 10, 13, 9, 0),
            ),
            (
                "2026-10-13T14:35",
                dt(2026, 10, 6, 14, 35),
                dt(2026, 10, 12, 14, 35),
                dt(2026, 10, 13, 13, 35),
                dt(2026, 10, 13, 14, 35),
            ),
        ];
        for (due, advance7, advance1, advance1h, due_local) in cases {
            let task = task("t", due);
            let result = plan(&task, dt(2026, 9, 1, 0, 0));
            assert_eq!(result.future_events.len(), 4, "due={due}");
            assert_eq!(
                event(&result, ReminderEventType::Advance7).scheduled_local,
                advance7,
                "due={due}"
            );
            assert_eq!(
                event(&result, ReminderEventType::Advance1).scheduled_local,
                advance1,
                "due={due}"
            );
            assert_eq!(
                event(&result, ReminderEventType::Advance1h).scheduled_local,
                advance1h,
                "due={due}"
            );
            assert_eq!(
                event(&result, ReminderEventType::Due).scheduled_local,
                due_local,
                "due={due}"
            );
            assert!(result
                .future_events
                .iter()
                .all(|event| event.key.canonical_due == due));
            assert_eq!(
                event(&result, ReminderEventType::Advance7).list_file,
                "t.yaml"
            );
        }
    }

    #[test]
    fn advance_uses_local_calendar_days_across_month_and_leap_edges() {
        let cases = [
            // (due, advance7, advance1)
            (
                "2025-01-01T14:45",
                dt(2024, 12, 25, 14, 45),
                dt(2024, 12, 31, 14, 45),
            ),
            (
                "2024-03-01",
                dt(2024, 2, 23, 9, 0),
                dt(2024, 2, 29, 9, 0),
            ),
        ];
        for (due, advance7, advance1) in cases {
            let task = task("t", due);
            let due_local = parse_due(due).unwrap().1;
            let result = plan(&task, due_local - Duration::days(60));
            assert_eq!(
                event(&result, ReminderEventType::Advance7).scheduled_local,
                advance7,
                "due={due}"
            );
            assert_eq!(
                event(&result, ReminderEventType::Advance1).scheduled_local,
                advance1,
                "due={due}"
            );
        }
    }

    #[test]
    fn dst_gap_moves_to_next_valid_minute_and_ambiguous_uses_earliest() {
        let requested = dt(2026, 3, 8, 2, 30);
        let next = dt(2026, 3, 8, 3, 0);
        let mut resolver = FakeResolver::default();
        for minute in 30..60 {
            resolver.map(dt(2026, 3, 8, 2, minute), LocalResolution::None);
        }
        resolver.map(next, LocalResolution::Single(utc(dt(2026, 3, 8, 7, 0))));
        let event = make_event(
            &task("gap", "2026-03-08T02:30"),
            "2026-03-08T02:30",
            ReminderEventType::Due,
            requested,
            &resolver,
        )
        .unwrap();
        assert_eq!(event.scheduled_local, next);
        assert_eq!(event.scheduled_at, utc(dt(2026, 3, 8, 7, 0)));

        let repeated = dt(2026, 11, 1, 1, 30);
        let early = utc(dt(2026, 11, 1, 5, 30));
        let late = utc(dt(2026, 11, 1, 6, 30));
        let mut resolver = FakeResolver::default();
        resolver.map(repeated, LocalResolution::Ambiguous(late, early));
        let event = make_event(
            &task("repeat", "2026-11-01T01:30"),
            "2026-11-01T01:30",
            ReminderEventType::Due,
            repeated,
            &resolver,
        )
        .unwrap();
        assert_eq!(event.scheduled_at, early);
    }

    #[test]
    fn dst_tiers_resolve_gap_and_ambiguous_for_1h_1d_7d() {
        let mut resolver = FakeResolver::default();
        // Spring-forward gap: 2026-03-08 02:30..02:59 do not exist; 03:00 does.
        for minute in 30..60 {
            resolver.map(dt(2026, 3, 8, 2, minute), LocalResolution::None);
        }
        resolver.map(dt(2026, 3, 8, 3, 0), LocalResolution::Single(utc(dt(2026, 3, 8, 7, 0))));
        // Fall-back ambiguity: 2026-11-01 01:30 occurs twice; earliest is chosen.
        let early = utc(dt(2026, 11, 1, 5, 30));
        let late = utc(dt(2026, 11, 1, 6, 30));
        resolver.map(dt(2026, 11, 1, 1, 30), LocalResolution::Ambiguous(late, early));

        // -1h tier lands in the gap → next valid minute (03:00).
        let gap_1h = task("gap1h", "2026-03-08T03:30");
        let plan = reconcile(
            std::slice::from_ref(&gap_1h),
            &ReminderLedger::default(),
            utc(dt(2026, 2, 28, 0, 0)),
            &resolver,
        );
        let advance1h = event(&plan, ReminderEventType::Advance1h);
        assert_eq!(advance1h.scheduled_local, dt(2026, 3, 8, 3, 0));
        assert_eq!(advance1h.scheduled_at, utc(dt(2026, 3, 8, 7, 0)));

        // -1d calendar rollback across the gap → next valid minute (03:00).
        let gap_1d = task("gap1d", "2026-03-09T02:30");
        let plan = reconcile(
            std::slice::from_ref(&gap_1d),
            &ReminderLedger::default(),
            utc(dt(2026, 3, 1, 0, 0)),
            &resolver,
        );
        assert_eq!(
            event(&plan, ReminderEventType::Advance1).scheduled_local,
            dt(2026, 3, 8, 3, 0)
        );

        // -1d calendar rollback across the fall-back → earliest occurrence.
        let amb_1d = task("amb1d", "2026-11-02T01:30");
        let plan = reconcile(
            std::slice::from_ref(&amb_1d),
            &ReminderLedger::default(),
            utc(dt(2026, 10, 25, 0, 0)),
            &resolver,
        );
        let advance1 = event(&plan, ReminderEventType::Advance1);
        assert_eq!(advance1.scheduled_local, dt(2026, 11, 1, 1, 30));
        assert_eq!(advance1.scheduled_at, early);

        // -7d calendar rollback across the fall-back → earliest occurrence.
        let amb_7d = task("amb7d", "2026-11-08T01:30");
        let plan = reconcile(
            std::slice::from_ref(&amb_7d),
            &ReminderLedger::default(),
            utc(dt(2026, 10, 31, 0, 0)),
            &resolver,
        );
        let advance7 = event(&plan, ReminderEventType::Advance7);
        assert_eq!(advance7.scheduled_local, dt(2026, 11, 1, 1, 30));
        assert_eq!(advance7.scheduled_at, early);
    }

    #[test]
    fn overdue_events_catch_up_once_and_claim_all_overdue_events() {
        let task = task("t", "2026-10-13T09:00");
        let result = plan(&task, dt(2026, 10, 14, 10, 0));
        assert_eq!(result.ready_notifications.len(), 1);
        assert_eq!(
            result.ready_notifications[0].event.key.event_type,
            ReminderEventType::Due
        );
        assert_eq!(result.claimed_ledger_keys.len(), 4);
        assert_eq!(result.ready_notifications[0].claim_keys.len(), 4);
        assert_eq!(
            result.ready_notifications[0].body.as_deref(),
            Some("已到截止时间")
        );
        assert_eq!(result.ready_notifications[0].event.task_title, "title t");
        assert!(result.future_events.is_empty());

        let mut ledger = ReminderLedger::default();
        ledger.claim_plan(&result);
        let repeated = reconcile(
            &[task],
            &ledger,
            utc(dt(2026, 10, 14, 10, 0)),
            &FakeResolver::default(),
        );
        assert!(repeated.ready_notifications.is_empty());
        assert!(repeated.claimed_ledger_keys.is_empty());
    }

    #[test]
    fn advance_overdue_merges_and_uses_real_remaining_copy() {
        let task = task("t", "2026-10-13T09:00");
        // now = due - 23h: advance7 and advance1 are past, advance1h and due future.
        let result = plan(&task, dt(2026, 10, 12, 10, 0));
        assert_eq!(result.ready_notifications.len(), 1);
        assert_eq!(
            result.ready_notifications[0].event.key.event_type,
            ReminderEventType::Advance1
        );
        assert_eq!(result.claimed_ledger_keys.len(), 2);
        assert_eq!(result.ready_notifications[0].claim_keys.len(), 2);
        assert_eq!(
            result.ready_notifications[0].body.as_deref(),
            Some("还有 23 小时到期")
        );
        assert_eq!(result.future_events.len(), 2);
        assert_eq!(result.next_due, Some(utc(dt(2026, 10, 13, 8, 0))));
    }

    #[test]
    fn only_earliest_advance_overdue_uses_day_copy_from_real_remaining() {
        let task = task("t", "2026-10-20T09:00");
        // now = advance7 + 1h: only advance7 is past.
        let result = plan(&task, dt(2026, 10, 13, 10, 0));
        assert_eq!(result.ready_notifications.len(), 1);
        assert_eq!(
            result.ready_notifications[0].event.key.event_type,
            ReminderEventType::Advance7
        );
        assert_eq!(result.claimed_ledger_keys.len(), 1);
        // due - now = 6d23h → floor(167 / 24) = 6.
        assert_eq!(
            result.ready_notifications[0].body.as_deref(),
            Some("还有 6 天到期")
        );
    }

    #[test]
    fn on_time_single_event_defers_to_tier_copy() {
        let task = task("t", "2026-10-20T09:00");
        // Fires 30s after advance7's planned moment → on time (within drift).
        let now = dt(2026, 10, 13, 9, 0) + Duration::seconds(30);
        let result = plan(&task, now);
        assert_eq!(result.ready_notifications.len(), 1);
        assert_eq!(
            result.ready_notifications[0].event.key.event_type,
            ReminderEventType::Advance7
        );
        assert_eq!(result.ready_notifications[0].body, None);
    }

    #[test]
    fn exactly_twenty_four_hours_remaining_uses_one_day_copy() {
        let task = task("t", "2026-10-13T09:00");
        // now = due - 24h: advance7 and advance1 (at exactly now) are past.
        let result = plan(&task, dt(2026, 10, 12, 9, 0));
        assert_eq!(result.ready_notifications.len(), 1);
        assert_eq!(
            result.ready_notifications[0].body.as_deref(),
            Some("还有 1 天到期")
        );
    }

    #[test]
    fn disabled_task_is_skipped_without_events() {
        let task = disabled(task("off", "2026-10-13"));
        let result = plan(&task, dt(2026, 10, 13, 10, 0));
        assert!(result.ready_notifications.is_empty());
        assert!(result.future_events.is_empty());
        assert!(result.claimed_ledger_keys.is_empty());
    }

    #[test]
    fn completed_or_unknown_status_is_skipped() {
        let mut completed = task("done", "2026-10-12");
        completed.status = "done".to_string();
        let mut unknown = task("unknown", "2026-10-12");
        unknown.status = "paused".to_string();
        let result = reconcile(
            &[completed, unknown],
            &ReminderLedger::default(),
            utc(dt(2026, 10, 13, 9, 0)),
            &FakeResolver::default(),
        );
        assert!(result.ready_notifications.is_empty());
        assert!(result.claimed_ledger_keys.is_empty());
    }

    #[test]
    fn changed_due_creates_new_key_but_changing_back_is_deduplicated() {
        let first = task("t", "2026-10-12");
        let first_plan = reconcile(
            std::slice::from_ref(&first),
            &ReminderLedger::default(),
            utc(dt(2026, 10, 13, 9, 0)),
            &FakeResolver::default(),
        );
        let mut ledger = ReminderLedger::default();
        ledger.claim_plan(&first_plan);

        let changed = task("t", "2026-10-14");
        let changed_plan = reconcile(
            &[changed],
            &ledger,
            utc(dt(2026, 10, 15, 9, 0)),
            &FakeResolver::default(),
        );
        assert_eq!(changed_plan.ready_notifications.len(), 1);
        assert_ne!(
            changed_plan.ready_notifications[0].event.key.canonical_due,
            first_plan.ready_notifications[0].event.key.canonical_due
        );
        ledger.claim_plan(&changed_plan);
        let back = reconcile(
            &[first],
            &ledger,
            utc(dt(2026, 10, 15, 9, 0)),
            &FakeResolver::default(),
        );
        assert!(back.ready_notifications.is_empty());
    }

    #[test]
    fn duplicate_task_ids_are_isolated_and_invalid_due_is_diagnosed() {
        let duplicate_a = ReminderTask {
            list_file: "a.yaml".into(),
            ..task("dup", "2026-10-12")
        };
        let duplicate_b = ReminderTask {
            list_file: "b.yaml".into(),
            ..task("dup", "2026-10-12")
        };
        let duplicate_same_list_a = task("same-list", "2026-10-12");
        let duplicate_same_list_b = task("same-list", "2026-10-12");
        let invalid = task("bad", "2026-02-30");
        let result = reconcile(
            &[
                duplicate_a,
                duplicate_b,
                duplicate_same_list_a,
                duplicate_same_list_b,
                invalid,
            ],
            &ReminderLedger::default(),
            utc(dt(2026, 10, 13, 10, 0)),
            &FakeResolver::default(),
        );
        assert_eq!(result.duplicate_task_ids, vec!["dup", "same-list"]);
        assert!(result.ready_notifications.is_empty());
        assert!(result.claimed_ledger_keys.is_empty());
        assert!(result.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            ReminderDiagnostic::InvalidDue { task_id, .. } if task_id == "bad"
        )));
    }

    #[test]
    fn first_frame_claims_past_advance_tiers_silently() {
        let task = task("t", "2026-10-13");
        let mut ledger = ReminderLedger::default();
        // now = due - ~23h: advance7 and advance1 are past; advance1h and due future.
        let now = utc(dt(2026, 10, 12, 10, 0));
        let outcome = observe_snapshot(
            std::slice::from_ref(&task),
            &mut ledger,
            now,
            &FakeResolver::default(),
        );
        assert_eq!(outcome.claimed_ledger_keys.len(), 2);
        assert!(outcome.changed);
        assert!(outcome.diagnostics.is_empty());
        assert_eq!(
            ledger.observed.get("t"),
            Some(&"2026-10-13".to_string())
        );

        let result = reconcile(
            std::slice::from_ref(&task),
            &ledger,
            now,
            &FakeResolver::default(),
        );
        assert!(result.ready_notifications.is_empty());
        assert_eq!(result.future_events.len(), 2);
        assert_eq!(
            result.future_events[0].key.event_type,
            ReminderEventType::Advance1h
        );
    }

    #[test]
    fn first_frame_leaves_past_due_for_normal_catch_up() {
        let task = task("t", "2026-10-10");
        let mut ledger = ReminderLedger::default();
        let now = utc(dt(2026, 10, 12, 10, 0));
        let outcome = observe_snapshot(
            std::slice::from_ref(&task),
            &mut ledger,
            now,
            &FakeResolver::default(),
        );
        assert_eq!(outcome.claimed_ledger_keys.len(), 3);
        assert_eq!(ledger.observed.get("t"), Some(&"2026-10-10".to_string()));

        let result = reconcile(
            std::slice::from_ref(&task),
            &ledger,
            now,
            &FakeResolver::default(),
        );
        assert_eq!(result.ready_notifications.len(), 1);
        assert_eq!(
            result.ready_notifications[0].event.key.event_type,
            ReminderEventType::Due
        );
        assert_eq!(
            result.ready_notifications[0].body.as_deref(),
            Some("已到截止时间")
        );
    }

    #[test]
    fn first_frame_marker_lifecycle_disable_reenable_and_prune() {
        let task = task("t", "2026-10-13");
        let now = utc(dt(2026, 10, 12, 10, 0));
        let mut ledger = ReminderLedger::default();

        // First observe seeds the marker and silently claims past advance tiers.
        observe_snapshot(
            std::slice::from_ref(&task),
            &mut ledger,
            now,
            &FakeResolver::default(),
        );
        assert!(ledger.observed.contains_key("t"));

        // Disabling the reminder removes the marker.
        let off = disabled(task.clone());
        let outcome = observe_snapshot(
            std::slice::from_ref(&off),
            &mut ledger,
            now,
            &FakeResolver::default(),
        );
        assert!(outcome.changed);
        assert!(!ledger.observed.contains_key("t"));

        // Re-enabling re-seeds the marker; already-attempted past tiers stay silent.
        let outcome = observe_snapshot(
            std::slice::from_ref(&task),
            &mut ledger,
            now,
            &FakeResolver::default(),
        );
        assert!(ledger.observed.contains_key("t"));
        assert!(outcome.claimed_ledger_keys.is_empty());
        let result = reconcile(
            std::slice::from_ref(&task),
            &ledger,
            now,
            &FakeResolver::default(),
        );
        assert!(result.ready_notifications.is_empty());

        // Disappearance prunes the marker.
        let outcome = observe_snapshot(&[], &mut ledger, now, &FakeResolver::default());
        assert!(outcome.changed);
        assert!(ledger.observed.is_empty());
    }

    #[test]
    fn first_frame_reseeds_on_due_change() {
        let original = task("t", "2026-10-13");
        let now = utc(dt(2026, 10, 12, 10, 0));
        let mut ledger = ReminderLedger::default();
        observe_snapshot(
            std::slice::from_ref(&original),
            &mut ledger,
            now,
            &FakeResolver::default(),
        );

        // Change due → canonical differs → first frame for the new due.
        let changed = task("t", "2026-10-14");
        let outcome = observe_snapshot(
            std::slice::from_ref(&changed),
            &mut ledger,
            now,
            &FakeResolver::default(),
        );
        assert!(!outcome.claimed_ledger_keys.is_empty());
        assert_eq!(ledger.observed.get("t"), Some(&"2026-10-14".to_string()));

        let result = reconcile(
            std::slice::from_ref(&changed),
            &ledger,
            now,
            &FakeResolver::default(),
        );
        assert!(result
            .ready_notifications
            .iter()
            .all(|ready| ready.event.key.event_type != ReminderEventType::Advance7));
    }

    #[test]
    fn observed_marker_never_set_for_invalid_or_absent_due() {
        let mut ledger = ReminderLedger::default();
        ledger.observed.insert("old".to_string(), "2026-10-01".to_string());
        let invalid = task("invalid", "2026-02-30");
        let no_due = ReminderTask {
            due: None,
            ..task("no-due", "2026-10-13")
        };
        let outcome = observe_snapshot(
            &[invalid, no_due],
            &mut ledger,
            utc(dt(2026, 10, 12, 10, 0)),
            &FakeResolver::default(),
        );
        assert!(ledger.observed.is_empty());
        assert!(outcome.claimed_ledger_keys.is_empty());
        assert!(outcome.changed);
    }

    #[test]
    fn legacy_advance_key_is_deserializable_and_tolerated() {
        let key: ReminderEventKey = serde_json::from_str(
            r#"{"task_id":"t","canonical_due":"2026-10-13","event_type":"advance"}"#,
        )
        .unwrap();
        assert_eq!(key.event_type, ReminderEventType::Advance);
        let encoded = serde_json::to_value(&key).unwrap();
        assert_eq!(encoded["event_type"], "advance");

        let mut ledger = ReminderLedger::default();
        ledger.attempted.insert(key);
        let task = task("t", "2026-10-13");
        let result = reconcile(
            std::slice::from_ref(&task),
            &ledger,
            utc(dt(2026, 10, 12, 10, 0)),
            &FakeResolver::default(),
        );
        // The legacy key never matches a new tier; future tiers are still scheduled.
        assert!(result
            .future_events
            .iter()
            .any(|event| event.key.event_type == ReminderEventType::Advance1h));
    }

    #[test]
    fn old_ledger_without_observed_remains_readable() {
        let legacy: ReminderLedger =
            serde_json::from_str(r#"{"last_status":{"t":true},"attempted":[]}"#).unwrap();
        assert!(legacy.observed.is_empty());
        assert_eq!(legacy.last_status.get("t"), Some(&true));
        assert!(legacy.attempted.is_empty());
        let encoded = serde_json::to_value(&legacy).unwrap();
        assert_eq!(encoded.get("observed"), Some(&serde_json::json!({})));
        assert_eq!(encoded.get("last_status"), Some(&serde_json::json!({"t": true})));
    }

    #[test]
    fn reopen_transition_claims_all_overdue_keys_silently() {
        let task = task("t", "2026-10-10");
        let mut ledger = ReminderLedger::default();
        ledger.last_status.insert("t".to_string(), false);
        let now = utc(dt(2026, 10, 12, 10, 0));
        let outcome = observe_snapshot(
            std::slice::from_ref(&task),
            &mut ledger,
            now,
            &FakeResolver::default(),
        );
        // advance7 / advance1 / advance1h + due are all overdue and claimed silently.
        assert_eq!(outcome.claimed_ledger_keys.len(), 4);
        assert_eq!(ledger.attempted.len(), 4);
        assert!(ledger
            .attempted
            .iter()
            .any(|key| key.event_type == ReminderEventType::Due));
        assert_eq!(ledger.last_status.get("t"), Some(&true));

        let plan = reconcile(
            std::slice::from_ref(&task),
            &ledger,
            now,
            &FakeResolver::default(),
        );
        assert!(plan.ready_notifications.is_empty());
        assert!(plan.future_events.is_empty());
    }

    #[test]
    fn non_open_snapshot_updates_last_status_and_clears_observed() {
        let mut done = task("t", "2026-10-10");
        done.status = "done".to_string();
        let mut ledger = ReminderLedger::default();
        ledger
            .observed
            .insert("t".to_string(), "2026-10-10".to_string());
        let outcome = observe_snapshot(
            std::slice::from_ref(&done),
            &mut ledger,
            utc(dt(2026, 10, 12, 10, 0)),
            &FakeResolver::default(),
        );
        assert!(ledger.observed.is_empty());
        assert_eq!(ledger.last_status.get("t"), Some(&false));
        assert!(outcome.claimed_ledger_keys.is_empty());
        assert!(outcome.changed);
    }

    #[test]
    fn absent_task_prunes_both_checkpoints() {
        let mut ledger = ReminderLedger::default();
        ledger
            .observed
            .insert("gone".to_string(), "2000-01-01".to_string());
        ledger.last_status.insert("gone".to_string(), true);
        let outcome = observe_snapshot(
            &[],
            &mut ledger,
            utc(dt(2026, 10, 12, 10, 0)),
            &FakeResolver::default(),
        );
        assert!(ledger.observed.is_empty());
        assert!(ledger.last_status.is_empty());
        assert!(outcome.changed);
    }

    #[test]
    fn resolution_search_is_bounded_and_failure_is_diagnosed() {
        let requested = dt(2026, 3, 8, 2, 30);
        let resolver = NeverResolver {
            calls: Cell::new(0),
        };
        assert!(resolve_wall_time(requested, &resolver).is_none());
        assert_eq!(resolver.calls.get(), MAX_LOCAL_RESOLUTION_MINUTES + 1);

        let mut boundary_resolver = FakeResolver::default();
        for minute in 0..MAX_LOCAL_RESOLUTION_MINUTES {
            boundary_resolver.map(
                requested + Duration::minutes(minute as i64),
                LocalResolution::None,
            );
        }
        let boundary = requested + Duration::minutes(MAX_LOCAL_RESOLUTION_MINUTES as i64);
        boundary_resolver.map(boundary, LocalResolution::Single(utc(boundary)));
        assert_eq!(
            resolve_wall_time(requested, &boundary_resolver),
            Some((boundary, utc(boundary)))
        );

        let resolver = NeverResolver {
            calls: Cell::new(0),
        };
        let result = reconcile(
            &[task("stuck", "2026-03-08T02:30")],
            &ReminderLedger::default(),
            utc(dt(2026, 3, 8, 0, 0)),
            &resolver,
        );
        assert!(result.ready_notifications.is_empty());
        assert!(result.future_events.is_empty());
        assert!(result.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            ReminderDiagnostic::LocalTimeUnresolvable {
                task_id,
                event_type: ReminderEventType::Due,
                ..
            } if task_id == "stuck"
        )));
    }

    #[test]
    fn minimum_chrono_date_does_not_panic_when_computing_advance_dates() {
        let minimum = NaiveDate::MIN.and_hms_opt(9, 0, 0).unwrap();
        assert!(matches!(
            advance_local_days("minimum", "due", minimum, 1),
            Err(ReminderDiagnostic::AdvanceDateOutOfRange { task_id, .. }) if task_id == "minimum"
        ));

        let earliest_contract_date = task("year-zero", "0000-01-01");
        let result = plan(&earliest_contract_date, dt(0, 1, 1, 8, 0));
        // No panic; the due tier is still resolved (future here) at 09:00.
        assert_eq!(event(&result, ReminderEventType::Due).scheduled_local.hour(), 9);
        assert!(result
            .future_events
            .iter()
            .any(|event| event.key.event_type == ReminderEventType::Due));
    }
}
