//! Phase 1 冻结的 Agent 命令（薄编排；逻辑在 `config` / `intent` / `sandbox` / `secret`）。
//!
//! **密钥绝不回传**；`agent_save_config` / `agent_set_key` **绝不**自动联网；
//! probe 仅在 `confirmPaid === true` 时才会走网络。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Notify;
use zeroize::Zeroizing;

use super::clock::Clock;
use super::provider::{CancelToken, Provider};
use super::record::{self, RunState, RunStats, RunSummary};
use super::runner::{self, RunDeps, RunInput, RunnerLock, StorageTaskAccess, TaskAccess};
use super::schedule::{self, QueueItem, Scheduler};
use super::{config, intent, provider, sandbox, secret};
use super::secret::SecretStore;
use crate::storage;

/// 结果根校验返回。
#[derive(Debug, Serialize)]
pub struct RootValidation {
    pub ok: bool,
    pub message: String,
}

/// probe 返回（`error_class` 缺省时省略）。
#[derive(Debug, Serialize)]
pub struct ProbeResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    pub message: String,
}

/// 读取 Agent 配置（含 `key_present`；不含密钥值）。
#[tauri::command]
pub fn agent_load_config() -> Result<config::AgentConfigView, String> {
    let base = storage::settings_dir()?;
    let cfg = config::load_agent_config_from(&base);
    let key_present = secret::DpapiSecretStore::new(&base).present();
    Ok(cfg.view(key_present))
}

/// 保存 Agent 配置：Value 补丁仅改 `agent` 已知字段；`execution_enabled` false→true 需 `consentCloud`。
#[tauri::command]
pub fn agent_save_config(
    config: serde_json::Value,
    consent_cloud: Option<bool>,
) -> Result<(), String> {
    let base = storage::settings_dir()?;
    super::config::save_agent_config_in(&base, &config, consent_cloud.unwrap_or(false))
}

/// 设置本机密钥（DPAPI 加密落独立文件；值永不回传）。入参以 `Zeroizing` 包裹，用后清空。
#[tauri::command]
pub fn agent_set_key(key: String) -> Result<(), String> {
    let base = storage::settings_dir()?;
    let key = Zeroizing::new(key);
    secret::DpapiSecretStore::new(&base).set(key.as_str())
}

/// 清除本机密钥（幂等）。
#[tauri::command]
pub fn agent_clear_key() -> Result<(), String> {
    let base = storage::settings_dir()?;
    secret::DpapiSecretStore::new(&base).clear()
}

/// 是否存在可用密钥（仅布尔）。
#[tauri::command]
pub fn agent_key_present() -> Result<bool, String> {
    let base = storage::settings_dir()?;
    Ok(secret::DpapiSecretStore::new(&base).present())
}

/// 校验结果根是否安全可写（不授权读取其中全部内容，仅写入探针）。
#[tauri::command]
pub fn agent_validate_result_root(path: String) -> RootValidation {
    match storage::settings_dir() {
        Err(e) => RootValidation {
            ok: false,
            message: e,
        },
        Ok(base) => match sandbox::validate_result_root_in(&base, &path) {
            Ok(()) => RootValidation {
                ok: true,
                message: "结果目录可用".to_string(),
            },
            Err(message) => RootValidation { ok: false, message },
        },
    }
}

/// 显式收费联网 probe：`confirm_paid` 为 false 时**不发网络**（fail closed）。
#[tauri::command]
pub async fn agent_probe(confirm_paid: bool) -> ProbeResult {
    probe_with_provider(confirm_paid, None).await
}

/// probe 核心（可注入 provider 供测试）。`confirm_paid == false` 时**不构建、不接触** provider。
///
/// provider 经统一工厂 [`provider::make_provider`] 构造：`agent-test-provider` 构建下返回离线
/// 假实现（只替换网络），默认 / release 返回真实 `DeepSeekProvider`。
pub(crate) async fn probe_with_provider(
    confirm_paid: bool,
    injected: Option<Arc<dyn provider::Provider>>,
) -> ProbeResult {
    if !confirm_paid {
        return ProbeResult {
            ok: false,
            error_class: Some(provider::CLASS_ABORTED.to_string()),
            message: "未确认费用，已取消（未发起网络请求）。".to_string(),
        };
    }
    let provider: Arc<dyn provider::Provider> = match injected {
        Some(p) => p,
        None => {
            let base = match storage::settings_dir() {
                Ok(base) => base,
                Err(message) => {
                    return ProbeResult {
                        ok: false,
                        error_class: Some(provider::CLASS_TRANSPORT.to_string()),
                        message,
                    }
                }
            };
            let cfg = config::load_agent_config_from(&base);
            let store: Arc<dyn secret::SecretStore> = Arc::new(secret::DpapiSecretStore::new(&base));
            if !store.present() {
                return ProbeResult {
                    ok: false,
                    error_class: Some(provider::CLASS_MISSING_CREDENTIAL.to_string()),
                    message: "未配置本机密钥。".to_string(),
                };
            }
            match provider::make_provider(store, cfg.model) {
                Ok(p) => p,
                Err(e) => {
                    return ProbeResult {
                        ok: false,
                        error_class: Some(e.class().to_string()),
                        message: e.message().to_string(),
                    }
                }
            }
        }
    };
    // 固定、无用户敏感内容的单 query；max_uses = 1、小 tokens。
    match provider
        .search(
            "connectivity check",
            1,
            provider::CancelToken::new(),
            Duration::from_secs(30),
        )
        .await
    {
        Ok(outcome) => ProbeResult {
            ok: true,
            error_class: None,
            message: format!("联网可用，返回 {} 条来源。", outcome.sources.len()),
        },
        Err(e) => ProbeResult {
            ok: false,
            error_class: Some(e.class().to_string()),
            message: e.message().to_string(),
        },
    }
}

/// `enabled: true` 前置检查，返回已配置的结果根。执行授权 / 密钥 / 结果根缺一即拒绝。
fn enable_preconditions(cfg: &config::AgentConfig, key_present: bool) -> Result<String, String> {
    if !cfg.execution_enabled {
        return Err("需先在本机配置中启用执行权限".to_string());
    }
    if !key_present {
        return Err("需先配置本机密钥".to_string());
    }
    cfg.result_root
        .clone()
        .ok_or_else(|| "需先配置结果目录".to_string())
}

/// 按 id 设置任务 Agent 意图；`enabled: true` 需本机执行授权 / 密钥 / 结果根齐备。
#[tauri::command]
pub fn agent_set_task_intent(
    list_file: String,
    task_id: String,
    enabled: bool,
    paths: Vec<String>,
) -> Result<(), String> {
    let base = storage::data_dir()?;
    if enabled {
        let settings = storage::settings_dir()?;
        let cfg = config::load_agent_config_from(&settings);
        let key_present = secret::DpapiSecretStore::new(&settings).present();
        let root = enable_preconditions(&cfg, key_present)?;
        sandbox::validate_result_root_in(&settings, &root)?;
    }
    intent::set_task_intent_in(&base, &list_file, &task_id, enabled, &paths)
}

/// 原生目录选择：返回路径字符串（不授予读取其中全部内容的权限）。
#[tauri::command]
pub async fn agent_pick_result_root(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().to_string())
}

/// 原生文件 / 目录选择：`directories = false` 多选文件、`true` 多选目录。
#[tauri::command]
pub async fn agent_pick_paths(app: tauri::AppHandle, directories: bool) -> Vec<String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = if directories {
        app.dialog().file().blocking_pick_folders()
    } else {
        app.dialog().file().blocking_pick_files()
    };
    picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 记录调用次数的假 provider；`confirm_paid=false` 时不应被触碰。
    struct SpyProvider {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl provider::Provider for SpyProvider {
        async fn chat(
            &self,
            _messages: serde_json::Value,
            _tools: serde_json::Value,
            _max_tokens: u32,
            _cancel: provider::CancelToken,
            _deadline: Duration,
        ) -> Result<provider::ChatOutcome, provider::ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(provider::ProviderError::new(
                provider::CLASS_ABORTED,
                "spy",
            ))
        }

        async fn search(
            &self,
            _query: &str,
            _max_uses: u32,
            _cancel: provider::CancelToken,
            _deadline: Duration,
        ) -> Result<provider::SearchOutcome, provider::ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(provider::ProviderError::new(
                provider::CLASS_ABORTED,
                "spy",
            ))
        }
    }

    #[tokio::test]
    async fn probe_without_consent_does_not_touch_provider() {
        let calls = Arc::new(AtomicUsize::new(0));
        let spy = SpyProvider {
            calls: calls.clone(),
        };
        let result = probe_with_provider(false, Some(Arc::new(spy))).await;
        assert!(!result.ok);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "confirm=false 不得接触 provider"
        );
        assert_eq!(result.error_class.as_deref(), Some(provider::CLASS_ABORTED));
    }

    #[test]
    fn enable_preconditions_require_config_key_and_root() {
        let mut cfg = config::AgentConfig::default();
        // execution_enabled 默认 false
        assert!(enable_preconditions(&cfg, false).is_err());
        cfg.execution_enabled = true;
        // 缺密钥
        assert!(enable_preconditions(&cfg, false).is_err());
        // 缺结果根
        assert!(enable_preconditions(&cfg, true).is_err());
        cfg.result_root = Some("D:\\r".to_string());
        assert_eq!(enable_preconditions(&cfg, true).unwrap(), "D:\\r");
    }
}

// ---------------------------------------------------------------------------
// Phase 2：运行编排 / 状态 / 结果（不改 P1 命令签名）
// ---------------------------------------------------------------------------

/// `agent_start_task` 返回（冻结字面 `{ runId }`）。
#[derive(Debug, Serialize)]
pub struct StartResult {
    #[serde(rename = "runId")]
    pub run_id: String,
}

/// 队列引用。
#[derive(Debug, Clone, Serialize)]
pub struct RunRef {
    pub run_id: String,
    pub list_file: String,
    pub task_id: String,
    pub attempt: u32,
}

/// `agent_get_status` 返回。
#[derive(Debug, Clone, Serialize)]
pub struct StatusResult {
    pub active: Option<RunSummary>,
    pub queue: Vec<RunRef>,
}

/// `agent://state` 事件载荷（camelCase；**绝不含密钥**）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateEvent {
    pub run_id: String,
    pub list_file: String,
    pub task_id: String,
    pub state: String,
    pub attempt: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
}

fn emit_state(app: &AppHandle, sum: &RunSummary) {
    let payload = StateEvent {
        run_id: sum.run_id.clone(),
        list_file: sum.list_file.clone(),
        task_id: sum.task_id.clone(),
        state: sum.state.clone(),
        attempt: sum.attempt,
        error_class: sum.error_class.clone(),
        stop_reason: sum.stop_reason.clone(),
        started_at: sum.started_at.clone(),
        ended_at: sum.ended_at.clone(),
    };
    let _ = app.emit("agent://state", payload);
}

/// 自动调度候选（纯函数，便于集成级测试）：夜窗内、启用、**未完成**、已存授权根合规、
/// **非本夜手动停止**、未超当夜 attempts 且最近记录状态可重排的任务。
pub(crate) fn plan_auto_queue(
    now_local: chrono::NaiveDateTime,
    now_ms: u64,
    lists: &[storage::ListView],
    records: &[record::RunRecord],
    manual_stops_today: &std::collections::HashSet<String>,
) -> Vec<QueueItem> {
    let date = schedule::night_date(now_local);
    let mut out = Vec::new();
    for list in lists {
        for task in &list.tasks {
            // 已完成任务不入队（避免空转）
            if task.status == "done" {
                continue;
            }
            let Some(agent) = &task.agent else { continue };
            if !agent.enabled {
                continue;
            }
            // 已存授权根复校验（外部 YAML 可绕过命令直接写 enabled:true）
            if agent
                .paths
                .iter()
                .any(|p| sandbox::guard_source_root(p).is_err())
            {
                continue;
            }
            let attempts = schedule::attempts_tonight(records, &task.id, date);
            let last = records
                .iter()
                .find(|r| r.task_id == task.id && r.list_file == list.file);
            let manual = manual_stops_today.contains(&task.id);
            if schedule::auto_eligible(now_local, true, attempts, manual, last).is_err() {
                continue;
            }
            let ready_at_ms = match last {
                Some(r) if r.state == RunState::Failed.as_str() => now_ms
                    .saturating_add(schedule::retry_delay(attempts.max(1)).as_millis() as u64),
                _ => now_ms,
            };
            out.push(QueueItem {
                run_id: ulid::Ulid::generate().to_string(),
                list_file: list.file.clone(),
                task_id: task.id.clone(),
                attempt: attempts + 1,
                ready_at_ms,
                force: false,
            });
        }
    }
    out
}

/// Agent 运行编排：独占锁 + 并发 1 队列 + 单飞执行 + 事件。
pub struct AgentRuntime {
    config_base: PathBuf,
    data_base: PathBuf,
    clock: Arc<dyn Clock>,
    lock: Option<RunnerLock>,
    active: Mutex<Option<RunSummary>>,
    queue: Mutex<Scheduler>,
    cancel: Mutex<Option<CancelToken>>,
    notify: Notify,
}

impl AgentRuntime {
    pub fn new(config_base: PathBuf, data_base: PathBuf, clock: Arc<dyn Clock>) -> Self {
        let lock = RunnerLock::acquire(&config_base).ok();
        Self {
            config_base,
            data_base,
            clock,
            lock,
            active: Mutex::new(None),
            queue: Mutex::new(Scheduler::new()),
            cancel: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    pub fn lock_held(&self) -> bool {
        self.lock.is_some()
    }

    /// 启动恢复 `running → interrupted`：**仅持有 runner.lock 时**执行，
    /// 避免第二实例把在跑（本实例）的记录误标 interrupted。
    pub fn recover_interrupted(&self, now_local: &str) -> Vec<String> {
        if !self.lock_held() {
            return Vec::new();
        }
        record::recover_interrupted_in(&self.config_base, now_local)
    }

    pub fn wake(&self) {
        self.notify.notify_one();
    }

    pub fn enqueue(&self, item: QueueItem) {
        self.queue.lock().unwrap().enqueue(item);
    }

    fn is_queued(&self, list_file: &str, task_id: &str) -> bool {
        self.queue
            .lock()
            .unwrap()
            .pending()
            .iter()
            .any(|i| i.list_file == list_file && i.task_id == task_id)
    }

    fn is_active(&self, list_file: &str, task_id: &str) -> bool {
        self.active
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|a| a.list_file == list_file && a.task_id == task_id)
    }

    /// 是否已在运行或排队（供 `agent_start_task` 去重，避免 JS 双击重复入队）。
    pub(crate) fn is_busy(&self, list_file: &str, task_id: &str) -> bool {
        self.is_queued(list_file, task_id) || self.is_active(list_file, task_id)
    }

    /// 取消：留**当夜手动停止**持久标记（本夜不自动重排）+ 移出队列 + 取消活动 run。
    /// 返回是否有影响。
    pub fn cancel_task(&self, list_file: &str, task_id: &str) -> bool {
        let date = schedule::night_date(self.clock.now_local())
            .format("%Y-%m-%d")
            .to_string();
        let _ = record::mark_manual_stop_in(&self.config_base, task_id, &date);

        let removed = self.queue.lock().unwrap().remove_task(list_file, task_id);
        let mut affected = removed > 0;
        let active_match = self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|a| a.list_file == list_file && a.task_id == task_id);
        if active_match {
            if let Some(tok) = self.cancel.lock().unwrap().as_ref() {
                tok.cancel();
            }
            affected = true;
        }
        affected
    }

    pub fn status(&self) -> StatusResult {
        let active = self.active.lock().unwrap().clone();
        let queue = self
            .queue
            .lock()
            .unwrap()
            .pending()
            .into_iter()
            .map(|q| RunRef {
                run_id: q.run_id,
                list_file: q.list_file,
                task_id: q.task_id,
                attempt: q.attempt,
            })
            .collect();
        StatusResult { active, queue }
    }

    /// 自动发现并排队夜窗内符合条件的启用任务（失败静默跳过，绝不崩）。
    fn enqueue_auto(&self) {
        let now_local = self.clock.now_local();
        if !schedule::in_window(now_local) {
            return;
        }
        let cfg = config::load_agent_config_from(&self.config_base);
        if !cfg.execution_enabled || cfg.result_root.is_none() {
            return;
        }
        if !secret::DpapiSecretStore::new(&self.config_base).present() {
            return;
        }
        let (records, _warnings) = record::list_records_in(&self.config_base);
        let Ok(loaded) = storage::load_lists_from(&self.data_base) else {
            return;
        };
        let date = schedule::night_date(now_local)
            .format("%Y-%m-%d")
            .to_string();
        let manual = record::manual_stops_on_in(&self.config_base, &date);
        let now_ms = self.clock.monotonic_ms();
        for item in plan_auto_queue(now_local, now_ms, &loaded.lists, &records, &manual) {
            if self.is_busy(&item.list_file, &item.task_id) {
                continue;
            }
            self.enqueue(item);
        }
    }

    /// 单飞：若无活动 run，弹出就绪项执行一次。
    async fn drain_once(&self, app: &AppHandle) {
        if !self.lock_held() || self.active.lock().unwrap().is_some() {
            return;
        }
        self.enqueue_auto();
        let now_ms = self.clock.monotonic_ms();
        let Some(item) = self.queue.lock().unwrap().pop_ready(now_ms) else {
            return;
        };
        // 09:00 后不再启动新 run（手动 force 除外）
        if !item.force && !schedule::in_window(self.clock.now_local()) {
            return;
        }
        let cfg = config::load_agent_config_from(&self.config_base);
        let Some(result_root) = cfg.result_root.clone() else {
            return;
        };
        let store: Arc<dyn SecretStore> = Arc::new(secret::DpapiSecretStore::new(&self.config_base));
        if !store.present() {
            return;
        }
        let provider: Arc<dyn Provider> = match provider::make_provider(store, cfg.model.clone()) {
            Ok(p) => p,
            Err(_) => return,
        };
        let cancel = CancelToken::new();
        *self.cancel.lock().unwrap() = Some(cancel.clone());
        let task: Arc<dyn TaskAccess> = Arc::new(StorageTaskAccess {
            data_base: self.data_base.clone(),
        });
        let deps = RunDeps {
            config_base: self.config_base.clone(),
            provider,
            clock: self.clock.clone(),
            model: cfg.model.clone(),
            result_root,
            max_uses: cfg.max_uses,
            task,
            cancel,
        };
        let input = RunInput {
            run_id: item.run_id.clone(),
            list_file: item.list_file.clone(),
            task_id: item.task_id.clone(),
            attempt: item.attempt,
        };
        let mut running = RunSummary {
            run_id: item.run_id.clone(),
            task_id: item.task_id.clone(),
            list_file: item.list_file.clone(),
            state: RunState::Running.as_str().to_string(),
            attempt: item.attempt,
            error_class: None,
            stop_reason: None,
            started_at: self.clock.now_local().format("%Y-%m-%dT%H:%M").to_string(),
            ended_at: None,
            result_dir: String::new(),
            partial: false,
            stats: RunStats::default(),
            model: cfg.model.clone(),
        };
        *self.active.lock().unwrap() = Some(running.clone());
        emit_state(app, &running);

        let outcome = runner::execute_run(&deps, &input).await;

        *self.active.lock().unwrap() = None;
        *self.cancel.lock().unwrap() = None;
        running.state = outcome.state.as_str().to_string();
        running.error_class = outcome.error_class.clone();
        running.stop_reason = outcome.stop_reason.clone();
        running.partial = outcome.partial;
        running.result_dir = outcome.result_dir.clone();
        running.stats = outcome.stats.clone();
        running.ended_at = Some(self.clock.now_local().format("%Y-%m-%dT%H:%M").to_string());
        emit_state(app, &running);
    }

    /// 后台工作线程：定时 + 唤醒触发 drain（应用运行期间）。
    pub fn spawn_worker(self: Arc<Self>, app: AppHandle) {
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    _ = self.notify.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                }
                self.drain_once(&app).await;
            }
        });
    }
}

/// 手动立即 / 重跑（`force` 绕夜窗）；仍计入当夜 ≤3 并尊重并发 1。
#[tauri::command]
pub async fn agent_start_task(
    app: AppHandle,
    list_file: String,
    task_id: String,
    force: bool,
) -> Result<StartResult, String> {
    let rt = app.state::<Arc<AgentRuntime>>().inner().clone();
    if !rt.lock_held() {
        return Err("本实例未持有 runner.lock，不执行 Agent".to_string());
    }
    let cfg = config::load_agent_config_from(&rt.config_base);
    let key_present = secret::DpapiSecretStore::new(&rt.config_base).present();
    let root = enable_preconditions(&cfg, key_present)?;
    sandbox::validate_result_root_in(&rt.config_base, &root)?;

    let task_access = StorageTaskAccess {
        data_base: rt.data_base.clone(),
    };
    let snap = task_access.snapshot(&list_file, &task_id)?;
    if !snap.enabled {
        return Err("任务未启用 Agent".to_string());
    }

    // 去重：同任务已在运行 / 排队 → 拒绝（避免 JS 双击重复入队）
    if rt.is_busy(&list_file, &task_id) {
        return Err("该任务已在运行或排队中".to_string());
    }

    let (records, _warnings) = record::list_records_in(&rt.config_base);
    let date = schedule::night_date(rt.clock.now_local());
    let attempts = schedule::attempts_tonight(&records, &task_id, date);
    if attempts >= schedule::MAX_ATTEMPTS_PER_NIGHT {
        return Err("本夜尝试次数已达上限".to_string());
    }

    // 显式 force（立即 / 重跑）解除本夜手动停止标记
    if force {
        let _ = record::clear_manual_stop_in(&rt.config_base, &task_id);
    }

    let run_id = ulid::Ulid::generate().to_string();
    rt.enqueue(QueueItem {
        run_id: run_id.clone(),
        list_file,
        task_id,
        attempt: attempts + 1,
        ready_at_ms: rt.clock.monotonic_ms(),
        force,
    });
    rt.wake();
    Ok(StartResult { run_id })
}

/// 停止：取消当前 run 与队列（保留既有成果；不自动重跑）。
#[tauri::command]
pub fn agent_stop_task(app: AppHandle, list_file: String, task_id: String) -> Result<(), String> {
    let rt = app.state::<Arc<AgentRuntime>>().inner().clone();
    rt.cancel_task(&list_file, &task_id);
    Ok(())
}

/// 当前活动 run + 队列。
#[tauri::command]
pub fn agent_get_status(app: AppHandle) -> Result<StatusResult, String> {
    let rt = app.state::<Arc<AgentRuntime>>().inner().clone();
    Ok(rt.status())
}

/// 列出某任务的历史运行（按 `started_at` 倒序分页）。
#[tauri::command]
pub fn agent_list_runs(
    app: AppHandle,
    list_file: String,
    task_id: String,
    offset: usize,
    limit: usize,
) -> Result<Vec<RunSummary>, String> {
    let rt = app.state::<Arc<AgentRuntime>>().inner().clone();
    let (records, _warnings) = record::list_records_in(&rt.config_base);
    Ok(records
        .iter()
        .filter(|r| r.list_file == list_file && r.task_id == task_id)
        .map(RunSummary::from)
        .skip(offset)
        .take(limit)
        .collect())
}

/// 打开结果目录：路径**仅**由记录内的 `result_dir` 推导，拒绝任意路径入参。
#[tauri::command]
pub fn agent_open_result_dir(app: AppHandle, run_id: String) -> Result<(), String> {
    let rt = app.state::<Arc<AgentRuntime>>().inner().clone();
    let rec = record::find_record_in(&rt.config_base, &run_id)
        .ok_or_else(|| "未找到运行记录".to_string())?;
    if rec.result_dir.is_empty() {
        return Err("该运行没有结果目录".to_string());
    }
    let dir = PathBuf::from(&rec.result_dir);
    if !dir.is_dir() {
        return Err("结果目录不存在".to_string());
    }
    app.opener()
        .open_path(rec.result_dir, None::<String>)
        .map_err(|e| format!("打开结果目录失败：{e}"))
}

#[cfg(test)]
mod p2_tests {
    use super::*;
    use crate::agent::clock::SystemClock;
    use crate::agent::record::{RunRecord, RunSnapshot, RunStats};
    use crate::agent::TaskAgent;
    use chrono::NaiveDate;
    use std::collections::HashSet;

    fn task(id: &str, status: &str, agent: Option<TaskAgent>) -> storage::TaskView {
        storage::TaskView {
            id: id.to_string(),
            title: "t".to_string(),
            status: status.to_string(),
            completed: None,
            priority: None,
            due: None,
            reminder_offset_days: None,
            tags: None,
            notes: None,
            subtasks: None,
            agent,
        }
    }

    fn list(file: &str, tasks: Vec<storage::TaskView>) -> storage::ListView {
        storage::ListView {
            name: "L".to_string(),
            file: file.to_string(),
            tasks,
        }
    }

    fn enabled_agent(paths: Vec<String>) -> Option<TaskAgent> {
        Some(TaskAgent {
            enabled: true,
            paths,
        })
    }

    fn rec(
        task_id: &str,
        state: RunState,
        class: Option<&str>,
        stop_reason: Option<&str>,
        started: &str,
    ) -> RunRecord {
        RunRecord {
            run_id: ulid::Ulid::generate().to_string(),
            task_id: task_id.to_string(),
            list_file: "L.yaml".to_string(),
            attempt: 1,
            state: state.as_str().to_string(),
            error_class: class.map(str::to_string),
            stop_reason: stop_reason.map(str::to_string),
            started_at: started.to_string(),
            ended_at: None,
            model: "m".to_string(),
            search_tool: "web_search_20250305".to_string(),
            stats: RunStats::default(),
            snapshot: RunSnapshot {
                title: "t".into(),
                notes: String::new(),
                paths: vec![],
            },
            result_dir: String::new(),
            partial: false,
        }
    }

    fn at(hour: u32) -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 12)
            .unwrap()
            .and_hms_opt(hour, 0, 0)
            .unwrap()
    }

    #[test]
    fn plan_skips_manual_stopped_tonight() {
        let lists = vec![list("L.yaml", vec![task("T1", "open", enabled_agent(vec![]))])];
        // 最近记录：本夜手动停止
        let records = vec![rec(
            "T1",
            RunState::Stopped,
            None,
            Some("manual"),
            "2026-10-12T01:00",
        )];
        let mut manual = HashSet::new();
        manual.insert("T1".to_string());

        assert!(
            plan_auto_queue(at(2), 0, &lists, &records, &manual).is_empty(),
            "本夜手动停止后不得被自动重排"
        );
        // 无手动标记（如已进入新夜）→ stopped 不长期阻断
        assert_eq!(
            plan_auto_queue(at(2), 0, &lists, &records, &HashSet::new()).len(),
            1
        );
    }

    #[test]
    fn plan_reenqueues_disabled_then_enabled() {
        let lists = vec![list("L.yaml", vec![task("T1", "open", enabled_agent(vec![]))])];
        // 最近记录：因停用而 stopped（无手动标记）
        let records = vec![rec(
            "T1",
            RunState::Stopped,
            None,
            Some("disabled"),
            "2026-10-12T01:00",
        )];
        let plan = plan_auto_queue(at(2), 0, &lists, &records, &HashSet::new());
        assert_eq!(plan.len(), 1, "停用→再启用的未成功任务应可重排");
        assert_eq!(plan[0].attempt, 2, "计入当夜 attempts 递增");
    }

    #[test]
    fn plan_skips_done_unsafe_disabled_capped_and_out_of_window() {
        // 已完成任务不入队
        let done = vec![list("L.yaml", vec![task("T1", "done", enabled_agent(vec![]))])];
        assert!(plan_auto_queue(at(2), 0, &done, &[], &HashSet::new()).is_empty());

        // 危险已存授权根 → 跳过
        let unsafe_list = vec![list(
            "L.yaml",
            vec![task("T1", "open", enabled_agent(vec!["C:\\".into()]))],
        )];
        assert!(plan_auto_queue(at(2), 0, &unsafe_list, &[], &HashSet::new()).is_empty());

        // 未启用 → 跳过
        let disabled = vec![list(
            "L.yaml",
            vec![task(
                "T1",
                "open",
                Some(TaskAgent {
                    enabled: false,
                    paths: vec![],
                }),
            )],
        )];
        assert!(plan_auto_queue(at(2), 0, &disabled, &[], &HashSet::new()).is_empty());

        // 当夜 attempts 已达 3 → 跳过
        let records = vec![
            rec("T1", RunState::Failed, Some("Transport"), None, "2026-10-12T00:10"),
            rec("T1", RunState::Failed, Some("Transport"), None, "2026-10-12T00:20"),
            rec("T1", RunState::Failed, Some("Transport"), None, "2026-10-12T00:30"),
        ];
        let lists = vec![list("L.yaml", vec![task("T1", "open", enabled_agent(vec![]))])];
        assert!(plan_auto_queue(at(2), 0, &lists, &records, &HashSet::new()).is_empty());

        // 窗口外（09:00 后）→ 跳过
        assert!(plan_auto_queue(at(10), 0, &lists, &[], &HashSet::new()).is_empty());

        // 合规 → 可入队
        assert_eq!(plan_auto_queue(at(2), 0, &lists, &[], &HashSet::new()).len(), 1);
    }

    #[test]
    fn runtime_lock_queue_and_manual_marker() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("cfg");
        let data = dir.path().join("data");
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let rt = AgentRuntime::new(cfg, data, clock);
        assert!(rt.lock_held());
        assert!(rt.status().active.is_none());
        assert!(rt.status().queue.is_empty());

        rt.enqueue(QueueItem {
            run_id: ulid::Ulid::generate().to_string(),
            list_file: "L".into(),
            task_id: "T".into(),
            attempt: 1,
            ready_at_ms: 0,
            force: true,
        });
        assert_eq!(rt.status().queue.len(), 1);
        // 去重：已排队 → busy
        assert!(rt.is_busy("L", "T"));
        assert!(!rt.is_busy("L", "OTHER"));

        // 取消队列项 → 记当夜手动停止标记
        assert!(rt.cancel_task("L", "T"));
        assert!(rt.status().queue.is_empty());
        let today = schedule::night_date(rt.clock.now_local())
            .format("%Y-%m-%d")
            .to_string();
        assert!(record::manual_stops_on_in(&rt.config_base, &today).contains("T"));
        // 再次取消无影响
        assert!(!rt.cancel_task("L", "T"));
    }

    #[test]
    fn second_instance_does_not_recover_running() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let running = rec("T1", RunState::Running, None, None, "2026-10-12T01:00");
        record::write_record_in(&cfg, &running).unwrap();

        let first = AgentRuntime::new(
            cfg.clone(),
            dir.path().join("d"),
            Arc::new(SystemClock::new()),
        );
        assert!(first.lock_held());

        // 第二实例：拿不到锁 → recover 空、记录保持 running（不被误标）
        let second = AgentRuntime::new(
            cfg.clone(),
            dir.path().join("d"),
            Arc::new(SystemClock::new()),
        );
        assert!(!second.lock_held());
        assert!(second.recover_interrupted("2026-10-12T09:30").is_empty());
        assert_eq!(
            record::find_record_in(&cfg, &running.run_id).unwrap().state,
            RunState::Running.as_str()
        );

        // 持锁实例可恢复
        assert_eq!(
            first.recover_interrupted("2026-10-12T09:30"),
            vec![running.run_id.clone()]
        );
        assert_eq!(
            record::find_record_in(&cfg, &running.run_id).unwrap().state,
            RunState::Interrupted.as_str()
        );
    }
}

