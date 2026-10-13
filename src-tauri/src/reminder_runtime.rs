//! Application-lifetime reminder adapter. Scheduling stays in `reminder`; this
//! module owns the worker, persisted at-most-once claims, and OS notification.
//!
//! Limitation: without a watcher or event log, an open -> done -> open sequence
//! that occurs entirely while the process is stopped cannot be reconstructed;
//! startup compares only the current snapshot with the last persisted status.

use std::path::{Path, PathBuf};
use std::time::Duration as StdDuration;
use std::{future::Future, sync::Arc};

use chrono::Utc;
use tauri::AppHandle;
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::reminder::{
    self, ReadyReminder, ReminderDiagnostic, ReminderEventType, ReminderLedger, ReminderPlan,
    ReminderTask, SystemLocalTimeResolver,
};
use crate::storage::{self, LoadResult};

const LEDGER_FILE: &str = "reminder-ledger.json";
const MAX_POLL_INTERVAL: StdDuration = StdDuration::from_secs(60);
const MIN_POLL_INTERVAL: StdDuration = StdDuration::from_millis(10);
const PAUSED_WARNING: &str = "清单未完整读取，提醒已暂停；全部清单读取成功后手动刷新即可恢复。";

#[derive(Clone)]
pub struct ReminderRuntime {
    sender: Option<mpsc::Sender<RefreshRequest>>,
    /// Serializes the whole disk-read -> worker-refresh transaction.
    load_lists_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Default)]
pub struct ReminderSnapshot {
    pub tasks: Vec<ReminderTask>,
    pub complete: bool,
    pub warnings: Vec<String>,
}

struct RefreshRequest {
    tasks: Vec<ReminderTask>,
    warnings: Vec<String>,
    complete_snapshot: bool,
    response: oneshot::Sender<Vec<String>>,
}

impl ReminderRuntime {
    pub fn start(app: AppHandle, settings_dir: PathBuf, initial: ReminderSnapshot) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        let initial_warnings = initial.warnings;
        tauri::async_runtime::spawn(run_worker(
            app,
            settings_dir.join(LEDGER_FILE),
            initial.tasks,
            initial.complete,
            initial_warnings,
            receiver,
        ));
        Self {
            sender: Some(sender),
            load_lists_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn disabled() -> Self {
        Self {
            sender: None,
            load_lists_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Replace the worker's snapshot and wait for one full reconciliation.
    pub async fn refresh(
        &self,
        tasks: Vec<ReminderTask>,
        warnings: Vec<String>,
        complete_snapshot: bool,
    ) -> Result<Vec<String>, String> {
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| "提醒运行时未启动".to_string())?;
        let (response, result) = oneshot::channel();
        sender
            .send(RefreshRequest {
                tasks,
                warnings,
                complete_snapshot,
                response,
            })
            .await
            .map_err(|_| "提醒运行时已停止".to_string())?;
        result
            .await
            .map_err(|_| "提醒运行时未能完成刷新".to_string())
    }

    /// Keep disk loading and worker acknowledgement in one serialized section;
    /// locking only the channel send would still allow stale reads to win.
    pub async fn load_lists(&self) -> Result<LoadResult, String> {
        load_lists_serialized(
            &self.load_lists_lock,
            || {
                let base = storage::data_dir()?;
                storage::ensure_data_dir(&base)?;
                storage::load_lists_from(&base)
            },
            |tasks, warnings, complete| self.refresh(tasks, warnings, complete),
        )
        .await
    }
}

async fn load_lists_serialized<L, F, Fut>(
    lock: &Mutex<()>,
    load: L,
    refresh: F,
) -> Result<LoadResult, String>
where
    L: FnOnce() -> Result<LoadResult, String>,
    F: FnOnce(Vec<ReminderTask>, Vec<String>, bool) -> Fut,
    Fut: Future<Output = Result<Vec<String>, String>>,
{
    let _guard = lock.lock().await;
    let mut loaded = match load() {
        Ok(loaded) => loaded,
        Err(error) => {
            // Even a hard directory/read error must stop the old schedule before
            // the command returns its error to the UI.
            let pause_result = refresh(Vec::new(), vec![PAUSED_WARNING.to_string()], false).await;
            let mut message = format!("{error}；{PAUSED_WARNING}");
            if let Err(refresh_error) = pause_result {
                message.push_str(&format!("（提醒暂停确认失败：{refresh_error}）"));
            }
            return Err(message);
        }
    };
    let complete = loaded.warnings.is_empty();
    let tasks = reminder_tasks(&loaded);
    if !complete
        && !loaded
            .warnings
            .iter()
            .any(|warning| warning == PAUSED_WARNING)
    {
        loaded.warnings.push(PAUSED_WARNING.to_string());
    }
    match refresh(tasks, Vec::new(), complete).await {
        Ok(warnings) => {
            for warning in warnings {
                if !loaded.warnings.contains(&warning) {
                    loaded.warnings.push(warning);
                }
            }
        }
        Err(error) => loaded.warnings.push(format!("提醒刷新失败：{error}")),
    }
    Ok(loaded)
}

pub fn reminder_tasks(loaded: &LoadResult) -> Vec<ReminderTask> {
    loaded
        .lists
        .iter()
        .flat_map(|list| {
            list.tasks.iter().map(|task| ReminderTask {
                list_file: list.file.clone(),
                task_id: task.id.clone(),
                title: task.title.clone(),
                status: task.status.clone(),
                due: task.due.clone(),
                reminder_enabled: task.reminder != Some(false),
            })
        })
        .collect()
}

trait LedgerStore {
    fn load(&self) -> Result<ReminderLedger, String>;
    fn save(&self, ledger: &ReminderLedger) -> Result<(), String>;
}

struct FileLedgerStore {
    path: PathBuf,
}

impl FileLedgerStore {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl LedgerStore for FileLedgerStore {
    fn load(&self) -> Result<ReminderLedger, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("提醒 ledger 损坏，已 fail closed：{error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ReminderLedger::default())
            }
            Err(error) => Err(format!("读取提醒 ledger 失败，已 fail closed：{error}")),
        }
    }

    fn save(&self, ledger: &ReminderLedger) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "提醒 ledger 路径无效".to_string())?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建提醒 ledger 目录失败：{error}"))?;
        let bytes = serde_json::to_vec_pretty(ledger)
            .map_err(|error| format!("序列化提醒 ledger 失败：{error}"))?;
        storage::atomic_write(&self.path, &bytes)
    }
}

trait Notifier {
    fn permission_granted(&self) -> Result<bool, String>;
    fn show(&self, ready: &ReadyReminder) -> Result<(), String>;
}

/// 「不再提醒」动作的 action type id，须与 `lib.rs` 注册值一致。
pub const REMINDER_ACTION_TYPE_ID: &str = "due-reminder";

struct TauriNotifier(AppHandle);

impl Notifier for TauriNotifier {
    fn permission_granted(&self) -> Result<bool, String> {
        self.0
            .notification()
            .permission_state()
            .map(|permission| permission == PermissionState::Granted)
            .map_err(|error| format!("查询系统通知权限失败：{error}"))
    }

    fn show(&self, ready: &ReadyReminder) -> Result<(), String> {
        let body = ready
            .body
            .clone()
            .unwrap_or_else(|| notification_body(ready.event.key.event_type).to_string());
        self.0
            .notification()
            .builder()
            .title(&ready.event.task_title)
            .body(body)
            .action_type_id(REMINDER_ACTION_TYPE_ID)
            .extra("taskId", ready.event.key.task_id.clone())
            .extra("listFile", ready.event.list_file.clone())
            .show()
            .map_err(|error| format!("显示系统通知失败：{error}"))
    }
}

/// 「不再提醒」动作回写：走与 `update_task_metadata` 命令相同的存储路径
/// （任务文件位于数据目录），写入 `reminder: false`。
pub fn disable_task_reminder(list_file: &str, task_id: &str) -> Result<(), String> {
    let base = storage::data_dir()?;
    disable_task_reminder_in(&base, list_file, task_id)
}

pub fn disable_task_reminder_in(base: &Path, list_file: &str, task_id: &str) -> Result<(), String> {
    let patch = storage::TaskMetadataPatch {
        reminder: storage::FieldPatch::Value(false),
        ..storage::TaskMetadataPatch::default()
    };
    storage::update_task_metadata_in(base, list_file, task_id, patch)
}

/// Claim and deliver one event per task. The store is updated before `show`,
/// and a failed `show` intentionally leaves the attempt recorded.
fn deliver_ready<L: LedgerStore, N: Notifier>(
    plan: &ReminderPlan,
    ledger: &mut ReminderLedger,
    store: &L,
    notifier: &N,
) -> Vec<String> {
    if plan.ready_notifications.is_empty() {
        return Vec::new();
    }
    match notifier.permission_granted() {
        Ok(true) => {}
        Ok(false) => return vec!["系统通知权限未授予，提醒未发送".to_string()],
        Err(error) => return vec![error],
    }

    let mut warnings = Vec::new();
    for ready in &plan.ready_notifications {
        let mut claimed = ledger.clone();
        claimed.claim(ready.claim_keys.iter().cloned());
        if let Err(error) = store.save(&claimed) {
            warnings.push(format!("写入提醒 ledger 失败，未发送通知：{error}"));
            break;
        }
        *ledger = claimed;
        if let Err(error) = notifier.show(ready) {
            warnings.push(error);
        }
    }
    warnings
}

fn notification_body(event_type: ReminderEventType) -> &'static str {
    match event_type {
        ReminderEventType::Advance7 => "还有 7 天到期",
        ReminderEventType::Advance1 => "还有 1 天到期",
        ReminderEventType::Advance1h => "还有 1 小时到期",
        ReminderEventType::Due => "已到截止时间",
        // 旧账本键仅用于读取兼容；不再生成，也不会被发送。
        ReminderEventType::Advance => "还有 1 天到期",
    }
}

async fn run_worker(
    app: AppHandle,
    ledger_path: PathBuf,
    mut tasks: Vec<ReminderTask>,
    complete_snapshot: bool,
    initial_warnings: Vec<String>,
    mut receiver: mpsc::Receiver<RefreshRequest>,
) {
    let store = FileLedgerStore::new(ledger_path);
    let mut ledger = store.load();
    let notifier = TauriNotifier(app);
    let mut snapshot_complete = complete_snapshot;
    let mut response: Option<oneshot::Sender<Vec<String>>> = None;
    let mut response_warnings = Vec::new();
    let mut background_warnings = initial_warnings;

    if snapshot_complete {
        background_warnings.extend(observe_snapshot(&tasks, &mut ledger, &store));
    } else if !background_warnings
        .iter()
        .any(|warning| warning == PAUSED_WARNING)
    {
        background_warnings.push(PAUSED_WARNING.to_string());
    }

    loop {
        let (plan, mut warnings) =
            reconcile_and_deliver(snapshot_complete, &tasks, &mut ledger, &store, &notifier);
        warnings.splice(0..0, std::mem::take(&mut response_warnings));

        if response.is_some() {
            warnings.append(&mut background_warnings);
        } else {
            for warning in warnings.drain(..) {
                if !background_warnings.contains(&warning) {
                    background_warnings.push(warning);
                }
            }
        }
        if let Some(reply) = response.take() {
            let _ = reply.send(warnings.clone());
        }

        let delay = next_poll_delay(plan.next_due);
        tokio::select! {
            request = receiver.recv() => match request {
                Some(request) => {
                    let mut refresh_warnings = request.warnings;
                    refresh_warnings.extend(accept_snapshot(
                        &mut tasks,
                        &mut snapshot_complete,
                        &request.tasks,
                        request.complete_snapshot,
                        &mut ledger,
                        &store,
                    ));
                    if request.complete_snapshot {
                        // A complete read is the only event that resumes the
                        // schedule after a partial/error pause.
                        background_warnings.retain(|warning| warning != PAUSED_WARNING);
                    }
                    response = Some(request.response);
                    response_warnings = refresh_warnings;
                }
                None => break,
            },
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

fn accept_snapshot<L: LedgerStore>(
    active_tasks: &mut Vec<ReminderTask>,
    snapshot_complete: &mut bool,
    incoming: &[ReminderTask],
    complete: bool,
    ledger: &mut Result<ReminderLedger, String>,
    store: &L,
) -> Vec<String> {
    if !complete {
        *snapshot_complete = false;
        return vec![PAUSED_WARNING.to_string()];
    }

    let warnings = observe_snapshot(incoming, ledger, store);
    *active_tasks = incoming.to_vec();
    *snapshot_complete = true;
    warnings
}

/// A partial or failed snapshot pauses all scheduling globally. The worker
/// retains the last complete task set but never reconciles it while paused.
fn reconcile_and_deliver<L: LedgerStore, N: Notifier>(
    snapshot_complete: bool,
    tasks: &[ReminderTask],
    ledger: &mut Result<ReminderLedger, String>,
    store: &L,
    notifier: &N,
) -> (ReminderPlan, Vec<String>) {
    if !snapshot_complete {
        return (ReminderPlan::default(), vec![PAUSED_WARNING.to_string()]);
    }

    let plan = reminder::reconcile(
        tasks,
        ledger.as_ref().unwrap_or(&ReminderLedger::default()),
        Utc::now(),
        &SystemLocalTimeResolver,
    );
    let mut warnings = diagnostic_warnings(&plan);
    match ledger {
        Ok(ledger) => warnings.extend(deliver_ready(&plan, ledger, store, notifier)),
        Err(error) => warnings.push(error.clone()),
    }
    (plan, warnings)
}

/// 观察一次完整快照：更新 `observed` 标记并静默记账首帧已过的提前档。
///
/// 只在完整快照时调用；部分快照不得触碰 `observed`（保持既有 fail-closed 暂停）。
fn observe_snapshot<L: LedgerStore>(
    tasks: &[ReminderTask],
    ledger: &mut Result<ReminderLedger, String>,
    store: &L,
) -> Vec<String> {
    let current = match ledger {
        Ok(current) => current,
        Err(error) => return vec![error.clone()],
    };

    let mut updated = current.clone();
    let outcome =
        reminder::observe_snapshot(tasks, &mut updated, Utc::now(), &SystemLocalTimeResolver);
    let mut warnings = diagnostic_lines(&outcome.diagnostics);

    if outcome.changed {
        if let Err(error) = store.save(&updated) {
            let failure = format!("写入提醒 ledger 失败，刷新已 fail closed：{error}");
            *ledger = Err(failure.clone());
            warnings.push(failure);
            return warnings;
        }
        *current = updated;
    }
    warnings
}

fn next_poll_delay(next_due: Option<chrono::DateTime<Utc>>) -> StdDuration {
    let Some(due) = next_due else {
        return MAX_POLL_INTERVAL;
    };
    match (due - Utc::now()).to_std() {
        Ok(until_due) => until_due.min(MAX_POLL_INTERVAL).max(MIN_POLL_INTERVAL),
        // 已过期（或负时长）→ 立即重查，而不是退避到 MAX，避免把迟到触发掩盖成按时。
        Err(_) => MIN_POLL_INTERVAL,
    }
}

#[cfg(test)]
mod next_poll_delay_tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn past_due_rechecks_immediately_and_future_is_clamped() {
        assert_eq!(next_poll_delay(None), MAX_POLL_INTERVAL);
        // 已过期 → 立即重查（原实现会退避 60s）。
        let past = Utc::now() - Duration::seconds(5);
        assert_eq!(next_poll_delay(Some(past)), MIN_POLL_INTERVAL);
        // 近期未来 → 取实际间隔（夹在 [MIN, MAX]）。
        let soon = Utc::now() + Duration::seconds(2);
        let delay = next_poll_delay(Some(soon));
        assert!(delay >= MIN_POLL_INTERVAL && delay < MAX_POLL_INTERVAL);
        // 远期未来 → 夹到 MAX。
        let far = Utc::now() + Duration::days(1);
        assert_eq!(next_poll_delay(Some(far)), MAX_POLL_INTERVAL);
    }
}

fn diagnostic_warnings(plan: &ReminderPlan) -> Vec<String> {
    diagnostic_lines(&plan.diagnostics)
}

fn diagnostic_lines(diagnostics: &[ReminderDiagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .map(|diagnostic| match diagnostic {
            ReminderDiagnostic::DuplicateTaskIds(ids) => {
                format!("重复 task id 已跳过提醒：{}", ids.join(", "))
            }
            ReminderDiagnostic::InvalidDue { task_id, due } => {
                format!("任务 {task_id} 的截止时间无效，已跳过提醒：{due}")
            }
            ReminderDiagnostic::AdvanceDateOutOfRange { task_id, due } => {
                format!("任务 {task_id} 无法计算提前提醒日期：{due}")
            }
            ReminderDiagnostic::LocalTimeUnresolvable {
                task_id,
                event_type,
                requested_local,
            } => format!(
                "任务 {task_id} 的本地提醒时刻无法解析（{event_type:?}）：{requested_local}"
            ),
        })
        .collect()
}

pub fn startup_snapshot(data_dir: &Path) -> ReminderSnapshot {
    match storage::load_lists_from(data_dir) {
        Ok(mut loaded) => {
            let complete = loaded.warnings.is_empty();
            if !complete
                && !loaded
                    .warnings
                    .iter()
                    .any(|warning| warning == PAUSED_WARNING)
            {
                loaded.warnings.push(PAUSED_WARNING.to_string());
            }
            ReminderSnapshot {
                tasks: reminder_tasks(&loaded),
                complete,
                warnings: loaded.warnings,
            }
        }
        Err(error) => ReminderSnapshot {
            tasks: Vec::new(),
            complete: false,
            warnings: vec![format!("启动时读取提醒快照失败：{error}；{PAUSED_WARNING}")],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reminder::{ReadyReminder, ReminderEvent, ReminderEventKey};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeStore {
        writes: Mutex<Vec<ReminderLedger>>,
        fail_write: bool,
        timeline: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    impl LedgerStore for FakeStore {
        fn load(&self) -> Result<ReminderLedger, String> {
            Ok(ReminderLedger::default())
        }

        fn save(&self, ledger: &ReminderLedger) -> Result<(), String> {
            if self.fail_write {
                return Err("disk full".to_string());
            }
            self.writes.lock().unwrap().push(ledger.clone());
            if let Some(timeline) = &self.timeline {
                timeline.lock().unwrap().push("ledger");
            }
            Ok(())
        }
    }

    struct FakeNotifier {
        permission: Result<bool, String>,
        show_error: bool,
        calls: Mutex<Vec<String>>,
        timeline: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    impl Default for FakeNotifier {
        fn default() -> Self {
            Self {
                permission: Ok(true),
                show_error: false,
                calls: Mutex::new(Vec::new()),
                timeline: None,
            }
        }
    }

    impl Notifier for FakeNotifier {
        fn permission_granted(&self) -> Result<bool, String> {
            self.permission.clone()
        }

        fn show(&self, ready: &ReadyReminder) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(ready.event.task_title.clone());
            if let Some(timeline) = &self.timeline {
                timeline.lock().unwrap().push("show");
            }
            if self.show_error {
                Err("toast unavailable".to_string())
            } else {
                Ok(())
            }
        }
    }

    fn ready_plan(event_type: ReminderEventType) -> ReminderPlan {
        let event = ReminderEvent {
            key: ReminderEventKey {
                task_id: "t1".to_string(),
                canonical_due: "2026-10-13".to_string(),
                event_type,
            },
            list_file: "list.yaml".to_string(),
            task_title: "Important task".to_string(),
            scheduled_local: chrono::NaiveDate::from_ymd_opt(2026, 10, 13)
                .unwrap()
                .and_hms_opt(9, 0, 0)
                .unwrap(),
            scheduled_at: Utc::now(),
        };
        let claim_keys = vec![event.key.clone()];
        ReminderPlan {
            ready_notifications: vec![ReadyReminder {
                event,
                claim_keys: claim_keys.clone(),
                body: None,
            }],
            claimed_ledger_keys: claim_keys,
            ..ReminderPlan::default()
        }
    }

    #[test]
    fn permission_denial_or_error_never_claims_or_shows() {
        for permission in [Ok(false), Err("permission query failed".to_string())] {
            let store = FakeStore::default();
            let notifier = FakeNotifier {
                permission,
                ..FakeNotifier::default()
            };
            let mut ledger = ReminderLedger::default();
            let warnings = deliver_ready(
                &ready_plan(ReminderEventType::Due),
                &mut ledger,
                &store,
                &notifier,
            );
            assert!(!warnings.is_empty());
            assert!(store.writes.lock().unwrap().is_empty());
            assert!(notifier.calls.lock().unwrap().is_empty());
            assert!(ledger.attempted.is_empty());
        }
    }

    #[test]
    fn ledger_failure_prevents_show() {
        let store = FakeStore {
            fail_write: true,
            ..FakeStore::default()
        };
        let notifier = FakeNotifier {
            permission: Ok(true),
            ..FakeNotifier::default()
        };
        let mut ledger = ReminderLedger::default();
        assert!(!deliver_ready(
            &ready_plan(ReminderEventType::Due),
            &mut ledger,
            &store,
            &notifier
        )
        .is_empty());
        assert!(notifier.calls.lock().unwrap().is_empty());
        assert!(ledger.attempted.is_empty());
    }

    #[test]
    fn persist_precedes_show_and_show_error_still_claims() {
        let timeline = Arc::new(Mutex::new(Vec::new()));
        let store = FakeStore {
            timeline: Some(timeline.clone()),
            ..FakeStore::default()
        };
        let notifier = FakeNotifier {
            permission: Ok(true),
            show_error: true,
            timeline: Some(timeline.clone()),
            ..FakeNotifier::default()
        };
        let mut ledger = ReminderLedger::default();
        let warnings = deliver_ready(
            &ready_plan(ReminderEventType::Advance1),
            &mut ledger,
            &store,
            &notifier,
        );
        assert_eq!(warnings, vec!["toast unavailable"]);
        assert_eq!(timeline.lock().unwrap().as_slice(), &["ledger", "show"]);
        assert_eq!(store.writes.lock().unwrap().len(), 1);
        assert_eq!(
            notifier.calls.lock().unwrap().as_slice(),
            &["Important task"]
        );
        assert_eq!(ledger.attempted.len(), 1);
        assert_eq!(
            notification_body(ReminderEventType::Advance7),
            "还有 7 天到期"
        );
        assert_eq!(
            notification_body(ReminderEventType::Advance1),
            "还有 1 天到期"
        );
        assert_eq!(
            notification_body(ReminderEventType::Advance1h),
            "还有 1 小时到期"
        );
        assert_eq!(
            notification_body(ReminderEventType::Due),
            "已到截止时间"
        );
    }

    #[test]
    fn corrupt_ledger_fails_closed_instead_of_becoming_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LEDGER_FILE);
        std::fs::write(&path, b"not json").unwrap();
        let store = FileLedgerStore::new(path);
        assert!(store.load().unwrap_err().contains("fail closed"));
    }

    #[test]
    fn persisted_catch_up_is_not_replayed_after_reload_or_duplicate_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileLedgerStore::new(dir.path().join(LEDGER_FILE));
        let task = ReminderTask {
            list_file: "list.yaml".to_string(),
            task_id: "t1".to_string(),
            title: "Persisted task".to_string(),
            status: "open".to_string(),
            due: Some("2000-01-01".to_string()),
            reminder_enabled: true,
        };
        let now = Utc::now();
        let mut ledger = store.load().unwrap();
        let plan = reminder::reconcile(
            std::slice::from_ref(&task),
            &ledger,
            now,
            &SystemLocalTimeResolver,
        );
        assert_eq!(plan.ready_notifications.len(), 1);
        let notifier = FakeNotifier {
            permission: Ok(true),
            ..FakeNotifier::default()
        };
        assert!(deliver_ready(&plan, &mut ledger, &store, &notifier).is_empty());
        assert_eq!(notifier.calls.lock().unwrap().len(), 1);

        // A new store instance represents a process restart; the same next
        // refresh must not send the already claimed catch-up again.
        let restarted_store = FileLedgerStore::new(dir.path().join(LEDGER_FILE));
        let mut restarted_ledger = restarted_store.load().unwrap();
        let refreshed =
            reminder::reconcile(&[task], &restarted_ledger, now, &SystemLocalTimeResolver);
        assert!(refreshed.ready_notifications.is_empty());
        assert!(deliver_ready(
            &refreshed,
            &mut restarted_ledger,
            &restarted_store,
            &notifier
        )
        .is_empty());
        assert_eq!(notifier.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn advance_catch_up_keeps_due_future_then_done_status_cancels_it() {
        let now_local = chrono::Local::now().naive_local();
        let due = (now_local + chrono::Duration::hours(12))
            .format("%Y-%m-%dT%H:%M")
            .to_string();
        let task = ReminderTask {
            list_file: "list.yaml".to_string(),
            task_id: "t1".to_string(),
            title: "Advance task".to_string(),
            status: "open".to_string(),
            due: Some(due),
            reminder_enabled: true,
        };
        let now = Utc::now();
        let mut ledger = ReminderLedger::default();
        let plan = reminder::reconcile(
            std::slice::from_ref(&task),
            &ledger,
            now,
            &SystemLocalTimeResolver,
        );
        // due in 12h: advance7 and advance1 are past; advance1h and due are future.
        assert_eq!(plan.ready_notifications.len(), 1);
        assert_eq!(
            plan.ready_notifications[0].event.key.event_type,
            ReminderEventType::Advance1
        );
        assert_eq!(plan.ready_notifications[0].claim_keys.len(), 2);
        assert_eq!(plan.future_events.len(), 2);
        assert!(plan
            .future_events
            .iter()
            .any(|event| event.key.event_type == ReminderEventType::Advance1h));
        assert!(plan
            .future_events
            .iter()
            .any(|event| event.key.event_type == ReminderEventType::Due));

        let dir = tempfile::tempdir().unwrap();
        let store = FileLedgerStore::new(dir.path().join(LEDGER_FILE));
        let notifier = FakeNotifier {
            permission: Ok(true),
            ..FakeNotifier::default()
        };
        assert!(deliver_ready(&plan, &mut ledger, &store, &notifier).is_empty());
        let later = now + chrono::Duration::days(2);
        let due_plan = reminder::reconcile(
            std::slice::from_ref(&task),
            &ledger,
            later,
            &SystemLocalTimeResolver,
        );
        assert_eq!(due_plan.ready_notifications.len(), 1);
        assert_eq!(
            due_plan.ready_notifications[0].event.key.event_type,
            ReminderEventType::Due
        );

        let mut done = task;
        done.status = "done".to_string();
        let completed_plan = reminder::reconcile(&[done], &ledger, later, &SystemLocalTimeResolver);
        assert!(completed_plan.ready_notifications.is_empty());
        assert!(completed_plan.future_events.is_empty());
    }

    #[test]
    fn reopening_seeds_past_advance_silently_but_keeps_future_due() {
        let due = (chrono::Local::now().naive_local() + chrono::Duration::hours(12))
            .format("%Y-%m-%dT%H:%M")
            .to_string();
        let reopened = ReminderTask {
            list_file: "list.yaml".to_string(),
            task_id: "reopen".to_string(),
            title: "Reopened task".to_string(),
            status: "open".to_string(),
            due: Some(due.clone()),
            reminder_enabled: true,
        };
        // No prior observed marker (e.g. previously done) → first frame.
        let mut ledger = Ok(ReminderLedger::default());
        let store = FakeStore::default();
        assert!(
            observe_snapshot(std::slice::from_ref(&reopened), &mut ledger, &store).is_empty()
        );
        let ledger = ledger.unwrap();
        assert_eq!(ledger.attempted.len(), 2);
        assert_eq!(ledger.observed.get("reopen"), Some(&due));
        let plan = reminder::reconcile(
            std::slice::from_ref(&reopened),
            &ledger,
            Utc::now(),
            &SystemLocalTimeResolver,
        );
        assert!(plan.ready_notifications.is_empty());
        assert_eq!(plan.future_events.len(), 2);
        assert!(plan
            .future_events
            .iter()
            .any(|event| event.key.event_type == ReminderEventType::Due));
    }

    fn past_due_task(id: &str, status: &str) -> ReminderTask {
        ReminderTask {
            list_file: "tasks.yaml".to_string(),
            task_id: id.to_string(),
            title: format!("Task {id}"),
            status: status.to_string(),
            due: Some("2000-01-01".to_string()),
            reminder_enabled: true,
        }
    }

    #[test]
    fn restart_seeds_without_observed_but_observed_keeps_normal_catch_up() {
        // Old ledger / first run (no observed marker): first frame seeds silently,
        // advancing the three past tiers and leaving the due for catch-up.
        let fresh = past_due_task("fresh", "open");
        let mut ledger = Ok(ReminderLedger::default());
        let store = FakeStore::default();
        assert!(observe_snapshot(std::slice::from_ref(&fresh), &mut ledger, &store).is_empty());
        let seeded = ledger.unwrap();
        assert_eq!(seeded.attempted.len(), 3);
        assert_eq!(seeded.observed.get("fresh"), Some(&"2000-01-01".to_string()));
        assert_eq!(store.writes.lock().unwrap().len(), 1);
        let plan = reminder::reconcile(&[fresh], &seeded, Utc::now(), &SystemLocalTimeResolver);
        assert_eq!(plan.ready_notifications.len(), 1);
        assert_eq!(plan.ready_notifications[0].event.key.event_type, ReminderEventType::Due);

        // A persisted observed marker does not re-seed: no silent claims, but the
        // normal catch-up still fires once.
        let mut existing = ReminderLedger::default();
        existing
            .observed
            .insert("fresh2".to_string(), "2000-01-01".to_string());
        let task2 = past_due_task("fresh2", "open");
        let mut ledger2 = Ok(existing.clone());
        assert!(
            observe_snapshot(std::slice::from_ref(&task2), &mut ledger2, &FakeStore::default())
                .is_empty()
        );
        assert!(ledger2.unwrap().attempted.is_empty());
        let normal =
            reminder::reconcile(&[task2], &existing, Utc::now(), &SystemLocalTimeResolver);
        assert_eq!(normal.ready_notifications.len(), 1);
    }

    #[test]
    fn duplicate_open_snapshot_is_isolated_and_does_not_seed() {
        let task = past_due_task("dup", "open");
        let mut ledger = Ok(ReminderLedger::default());
        let store = FakeStore::default();
        assert!(observe_snapshot(
            &[task.clone(), past_due_task("dup", "done")],
            &mut ledger,
            &store,
        )
        .is_empty());
        assert!(ledger.as_ref().unwrap().observed.is_empty());
        assert!(ledger.as_ref().unwrap().attempted.is_empty());
        assert!(store.writes.lock().unwrap().is_empty());

        // A unique snapshot then seeds and leaves the due catch-up.
        assert!(
            observe_snapshot(std::slice::from_ref(&task), &mut ledger, &store).is_empty()
        );
        assert_eq!(ledger.as_ref().unwrap().attempted.len(), 3);
        let plan = reminder::reconcile(
            &[task],
            ledger.as_ref().unwrap(),
            Utc::now(),
            &SystemLocalTimeResolver,
        );
        assert_eq!(plan.ready_notifications.len(), 1);
    }

    #[test]
    fn duplicate_is_isolated_and_done_then_open_reseeds() {
        let mut ledger = Ok(ReminderLedger::default());
        let store = FakeStore::default();
        let duplicate = [
            past_due_task("d", "open"),
            past_due_task("d", "done"),
        ];
        assert!(observe_snapshot(&duplicate, &mut ledger, &store).is_empty());
        assert!(ledger.as_ref().unwrap().observed.is_empty());
        assert!(ledger.as_ref().unwrap().last_status.is_empty());
        assert!(ledger.as_ref().unwrap().attempted.is_empty());
        assert!(store.writes.lock().unwrap().is_empty());

        // A done task records a closed checkpoint and carries no observed marker.
        assert!(
            observe_snapshot(&[past_due_task("d", "done")], &mut ledger, &store).is_empty()
        );
        assert!(ledger.as_ref().unwrap().observed.is_empty());
        assert_eq!(ledger.as_ref().unwrap().last_status.get("d"), Some(&false));

        // Reopen claims ALL overdue keys silently (advance tiers + due) → no catch-up.
        let reopened = past_due_task("d", "open");
        assert!(
            observe_snapshot(std::slice::from_ref(&reopened), &mut ledger, &store).is_empty()
        );
        let saved = ledger.unwrap();
        assert_eq!(saved.last_status.get("d"), Some(&true));
        assert_eq!(saved.attempted.len(), 4);
        assert!(saved
            .attempted
            .iter()
            .any(|key| key.event_type == ReminderEventType::Due));
        let plan = reminder::reconcile(&[reopened], &saved, Utc::now(), &SystemLocalTimeResolver);
        assert!(plan.ready_notifications.is_empty());
    }

    #[test]
    fn reopening_suppresses_past_keys_but_keeps_future_events() {
        let due = (chrono::Local::now().naive_local() + chrono::Duration::hours(12))
            .format("%Y-%m-%dT%H:%M")
            .to_string();
        let reopened = ReminderTask {
            list_file: "list.yaml".to_string(),
            task_id: "reopen".to_string(),
            title: "Reopened task".to_string(),
            status: "open".to_string(),
            due: Some(due),
            reminder_enabled: true,
        };
        // Prior complete snapshot observed this task as non-open (done).
        let mut prior = ReminderLedger::default();
        prior.last_status.insert("reopen".to_string(), false);
        let mut ledger = Ok(prior);
        let store = FakeStore::default();
        assert!(
            observe_snapshot(std::slice::from_ref(&reopened), &mut ledger, &store).is_empty()
        );
        let ledger = ledger.unwrap();
        // due in 12h: advance7 + advance1 are overdue and claimed on reopen.
        assert_eq!(ledger.attempted.len(), 2);
        assert!(ledger
            .attempted
            .iter()
            .all(|key| key.event_type != ReminderEventType::Due));
        assert_eq!(ledger.last_status.get("reopen"), Some(&true));
        let plan = reminder::reconcile(
            std::slice::from_ref(&reopened),
            &ledger,
            Utc::now(),
            &SystemLocalTimeResolver,
        );
        assert!(plan.ready_notifications.is_empty());
        assert_eq!(plan.future_events.len(), 2);
        assert!(plan
            .future_events
            .iter()
            .any(|event| event.key.event_type == ReminderEventType::Due));
    }

    #[test]
    fn restart_closed_checkpoint_suppresses_catch_up_but_open_checkpoint_keeps_it() {
        // Old ledger JSON on disk still carries last_status; it is honored on load.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LEDGER_FILE);
        let old_checkpoint = ReminderLedger {
            last_status: [("reopen".to_string(), false)].into_iter().collect(),
            ..ReminderLedger::default()
        };
        std::fs::write(&path, serde_json::to_vec(&old_checkpoint).unwrap()).unwrap();
        let restart_store = FileLedgerStore::new(path.clone());
        let mut restarted = Ok(restart_store.load().unwrap());
        let task = past_due_task("reopen", "open");
        let checkpoint_store = FakeStore::default();
        assert!(observe_snapshot(
            std::slice::from_ref(&task),
            &mut restarted,
            &checkpoint_store,
        )
        .is_empty());
        let persisted = restarted.unwrap();
        assert_eq!(persisted.last_status.get("reopen"), Some(&true));
        // Reopen claims ALL overdue keys (advance tiers + due) silently.
        assert_eq!(persisted.attempted.len(), 4);
        let writes = checkpoint_store.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].last_status.get("reopen"), Some(&true));
        assert_eq!(writes[0].attempted.len(), 4);
        assert!(writes[0]
            .attempted
            .iter()
            .any(|key| key.event_type == ReminderEventType::Due));
        drop(writes);
        let plan = reminder::reconcile(&[task], &persisted, Utc::now(), &SystemLocalTimeResolver);
        assert!(plan.ready_notifications.is_empty());
        assert!(plan.future_events.is_empty());

        // An open checkpoint (observed as open before) is NOT a reopen: normal
        // due catch-up still fires.
        let ordinary_open = past_due_task("ordinary", "open");
        let mut open_ledger = ReminderLedger::default();
        open_ledger.last_status.insert("ordinary".to_string(), true);
        assert!(observe_snapshot(
            std::slice::from_ref(&ordinary_open),
            &mut Ok(open_ledger.clone()),
            &FakeStore::default(),
        )
        .is_empty());
        let normal = reminder::reconcile(
            &[ordinary_open],
            &open_ledger,
            Utc::now(),
            &SystemLocalTimeResolver,
        );
        assert_eq!(normal.ready_notifications.len(), 1);
        assert_eq!(
            normal.ready_notifications[0].event.key.event_type,
            ReminderEventType::Due
        );
    }

    #[test]
    fn duplicate_open_snapshot_preserves_open_checkpoint_without_false_claim() {
        let task = past_due_task("dup", "open");
        let mut ledger = Ok(ReminderLedger::default());
        ledger
            .as_mut()
            .unwrap()
            .last_status
            .insert("dup".to_string(), true);
        let store = FakeStore::default();
        assert!(observe_snapshot(
            &[task.clone(), past_due_task("dup", "done")],
            &mut ledger,
            &store,
        )
        .is_empty());
        // Duplicate is ambiguous: checkpoint preserved, nothing claimed, no write.
        assert_eq!(ledger.as_ref().unwrap().last_status.get("dup"), Some(&true));
        assert!(ledger.as_ref().unwrap().attempted.is_empty());
        assert!(store.writes.lock().unwrap().is_empty());

        // A unique snapshot seeds the observed marker (no reopen suppression), and
        // the normal due catch-up still fires.
        assert!(
            observe_snapshot(std::slice::from_ref(&task), &mut ledger, &store).is_empty()
        );
        assert_eq!(ledger.as_ref().unwrap().attempted.len(), 3);
        assert!(ledger.as_ref().unwrap().observed.contains_key("dup"));
        let plan = reminder::reconcile(
            &[task],
            ledger.as_ref().unwrap(),
            Utc::now(),
            &SystemLocalTimeResolver,
        );
        assert_eq!(plan.ready_notifications.len(), 1);
    }

    #[test]
    fn reminder_disabled_task_yields_no_notifications() {
        let mut disabled = past_due_task("off", "open");
        disabled.reminder_enabled = false;
        let mut ledger = Ok(ReminderLedger::default());
        let store = FakeStore::default();
        assert!(
            observe_snapshot(std::slice::from_ref(&disabled), &mut ledger, &store).is_empty()
        );
        let saved = ledger.unwrap();
        // Disabled task: no observed marker, no claims, but still tracks open status.
        assert!(!saved.observed.contains_key("off"));
        assert!(saved.attempted.is_empty());
        assert_eq!(saved.last_status.get("off"), Some(&true));
        let plan = reminder::reconcile(
            std::slice::from_ref(&disabled),
            &saved,
            Utc::now(),
            &SystemLocalTimeResolver,
        );
        assert!(plan.ready_notifications.is_empty());
        assert!(plan.future_events.is_empty());
    }

    #[test]
    fn duplicate_after_done_keeps_closed_checkpoint_for_later_reopen() {
        let mut ledger = Ok(ReminderLedger::default());
        ledger
            .as_mut()
            .unwrap()
            .last_status
            .insert("dup-done".to_string(), false);
        let store = FakeStore::default();
        let duplicate = [
            past_due_task("dup-done", "open"),
            past_due_task("dup-done", "done"),
        ];
        assert!(observe_snapshot(&duplicate, &mut ledger, &store).is_empty());
        // Duplicate keeps the closed checkpoint and claims nothing.
        assert_eq!(
            ledger.as_ref().unwrap().last_status.get("dup-done"),
            Some(&false)
        );
        assert!(ledger.as_ref().unwrap().attempted.is_empty());

        let reopened = past_due_task("dup-done", "open");
        assert!(
            observe_snapshot(std::slice::from_ref(&reopened), &mut ledger, &store).is_empty()
        );
        let saved = ledger.unwrap();
        assert_eq!(saved.last_status.get("dup-done"), Some(&true));
        // Reopen claims all overdue keys silently → no catch-up notification.
        assert_eq!(saved.attempted.len(), 4);
        let plan = reminder::reconcile(&[reopened], &saved, Utc::now(), &SystemLocalTimeResolver);
        assert!(plan.ready_notifications.is_empty());
    }

    #[test]
    fn partial_snapshot_pauses_without_plan_and_complete_snapshot_resumes_replaced_tasks() {
        let old_task = past_due_task("old", "open");
        let partial_tasks = [
            past_due_task("duplicate", "open"),
            past_due_task("duplicate", "done"),
        ];
        let mut active_tasks = vec![old_task];
        let mut snapshot_complete = true;
        let mut ledger = Ok(ReminderLedger::default());
        ledger
            .as_mut()
            .unwrap()
            .observed
            .insert("unread-list-task".to_string(), "2000-01-01".to_string());
        let store = FakeStore::default();
        let notifier = FakeNotifier::default();

        let warnings = accept_snapshot(
            &mut active_tasks,
            &mut snapshot_complete,
            &partial_tasks,
            false,
            &mut ledger,
            &store,
        );
        assert_eq!(warnings, vec![PAUSED_WARNING]);
        assert!(!snapshot_complete);
        assert_eq!(active_tasks[0].task_id, "old");
        assert_eq!(
            ledger.as_ref().unwrap().observed.get("unread-list-task"),
            Some(&"2000-01-01".to_string())
        );
        assert!(store.writes.lock().unwrap().is_empty());

        let (paused_plan, paused_warnings) = reconcile_and_deliver(
            snapshot_complete,
            &active_tasks,
            &mut ledger,
            &store,
            &notifier,
        );
        assert!(paused_plan.ready_notifications.is_empty());
        assert!(paused_plan.future_events.is_empty());
        assert_eq!(paused_warnings, vec![PAUSED_WARNING]);
        assert!(notifier.calls.lock().unwrap().is_empty());

        let replacement = past_due_task("replacement", "open");
        assert!(accept_snapshot(
            &mut active_tasks,
            &mut snapshot_complete,
            std::slice::from_ref(&replacement),
            true,
            &mut ledger,
            &store,
        )
        .is_empty());
        assert!(snapshot_complete);
        assert_eq!(active_tasks, vec![replacement.clone()]);
        let (active_plan, active_warnings) = reconcile_and_deliver(
            snapshot_complete,
            &active_tasks,
            &mut ledger,
            &store,
            &notifier,
        );
        assert_eq!(active_plan.ready_notifications.len(), 1);
        assert!(active_warnings.is_empty());
        assert_eq!(
            notifier.calls.lock().unwrap().as_slice(),
            &["Task replacement"]
        );
        assert!(!ledger
            .as_ref()
            .unwrap()
            .observed
            .contains_key("unread-list-task"));
    }

    fn empty_load_result(warnings: Vec<String>) -> LoadResult {
        LoadResult {
            lists: Vec::new(),
            warnings,
        }
    }

    #[tokio::test]
    async fn load_and_worker_ack_are_serialized_with_disk_read() {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();

        let first_lock = lock.clone();
        let first_events = events.clone();
        let first_refresh_events = events.clone();
        let first = tokio::spawn(async move {
            load_lists_serialized(
                &first_lock,
                move || {
                    first_events.lock().unwrap().push("read-A");
                    Ok(empty_load_result(Vec::new()))
                },
                move |_tasks, _warnings, _complete| async move {
                    first_refresh_events.lock().unwrap().push("submit-A");
                    let _ = started_tx.send(());
                    let _ = release_rx.await;
                    first_refresh_events.lock().unwrap().push("ack-A");
                    Ok(Vec::new())
                },
            )
            .await
            .unwrap()
        });

        started_rx.await.unwrap();
        let second_lock = lock.clone();
        let second_events = events.clone();
        let second_refresh_events = events.clone();
        let second = tokio::spawn(async move {
            load_lists_serialized(
                &second_lock,
                move || {
                    second_events.lock().unwrap().push("read-B");
                    Ok(empty_load_result(Vec::new()))
                },
                move |_tasks, _warnings, _complete| async move {
                    second_refresh_events.lock().unwrap().push("submit-B");
                    second_refresh_events.lock().unwrap().push("ack-B");
                    Ok(Vec::new())
                },
            )
            .await
            .unwrap()
        });

        tokio::task::yield_now().await;
        assert_eq!(events.lock().unwrap().as_slice(), &["read-A", "submit-A"]);
        let _ = release_tx.send(());
        first.await.unwrap();
        second.await.unwrap();
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &["read-A", "submit-A", "ack-A", "read-B", "submit-B", "ack-B"]
        );
    }

    #[tokio::test]
    async fn hard_load_error_submits_pause_and_waits_for_ack_before_returning() {
        let lock = tokio::sync::Mutex::new(());
        let acknowledged = Arc::new(Mutex::new(false));
        let ack_for_refresh = acknowledged.clone();
        let result = load_lists_serialized(
            &lock,
            || Err("清单目录不可读".to_string()),
            move |tasks, warnings, complete| async move {
                assert!(tasks.is_empty());
                assert!(!complete);
                assert!(warnings.iter().any(|warning| warning == PAUSED_WARNING));
                tokio::task::yield_now().await;
                *ack_for_refresh.lock().unwrap() = true;
                Ok(vec![PAUSED_WARNING.to_string()])
            },
        )
        .await;

        assert!(result.as_ref().unwrap_err().contains("提醒已暂停"));
        assert!(*acknowledged.lock().unwrap());
    }

    #[tokio::test]
    async fn partial_load_result_reports_pause_and_submits_incomplete_snapshot() {
        let lock = tokio::sync::Mutex::new(());
        let request_complete = Arc::new(Mutex::new(None));
        let complete_for_refresh = request_complete.clone();
        let loaded = load_lists_serialized(
            &lock,
            || Ok(empty_load_result(vec!["读取清单失败".to_string()])),
            move |_tasks, _warnings, complete| async move {
                *complete_for_refresh.lock().unwrap() = Some(complete);
                Ok(vec![PAUSED_WARNING.to_string()])
            },
        )
        .await
        .unwrap();
        assert_eq!(*request_complete.lock().unwrap(), Some(false));
        assert!(loaded
            .warnings
            .iter()
            .any(|warning| warning == PAUSED_WARNING));
        assert!(loaded
            .warnings
            .iter()
            .any(|warning| warning == "读取清单失败"));
    }

    #[test]
    fn old_ledger_without_observed_loads_and_legacy_advance_keys_are_tolerated() {
        let old: ReminderLedger = serde_json::from_str(
            r#"{"last_status":{"t":true},"attempted":[{"task_id":"t","canonical_due":"2026-10-13","event_type":"advance"}]}"#,
        )
        .unwrap();
        assert!(old.observed.is_empty());
        assert_eq!(old.attempted.len(), 1);
        assert_eq!(
            old.attempted.iter().next().unwrap().event_type,
            ReminderEventType::Advance
        );
        let encoded = serde_json::to_value(&old).unwrap();
        assert_eq!(encoded.get("observed"), Some(&serde_json::json!({})));
        assert_eq!(encoded["attempted"][0]["event_type"], "advance");
    }

    #[test]
    fn disable_task_reminder_in_writes_false_and_rejects_unknown_without_due() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let lists = base.join(storage::LISTS_DIR);
        std::fs::create_dir_all(&lists).unwrap();
        std::fs::write(
            lists.join("L.yaml"),
            "tasks:\n  - id: T1\n    title: T\n    due: '2026-10-13'\n    reminder_offset_days: 1\n  - id: T2\n    title: T2\n",
        )
        .unwrap();

        disable_task_reminder_in(base, "L.yaml", "T1").unwrap();
        let text = std::fs::read_to_string(lists.join("L.yaml")).unwrap();
        let value: yaml_serde::Value = yaml_serde::from_str(&text).unwrap();
        let task = &value.get("tasks").and_then(yaml_serde::Value::as_sequence).unwrap()[0];
        assert_eq!(task.get("reminder").and_then(yaml_serde::Value::as_bool), Some(false));
        assert!(task.get("reminder_offset_days").is_none());

        // 无 due 的任务无法写入 reminder:false → 报错（调用方只记日志）。
        assert!(disable_task_reminder_in(base, "L.yaml", "T2").is_err());
        assert!(disable_task_reminder_in(base, "L.yaml", "NOPE").is_err());
    }
}
