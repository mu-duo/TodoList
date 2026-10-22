//! 运行记录事实源：`<config>/agent/runs/{run_id}.json`，一次运行一文件，temp+rename 原子写。
//!
//! 列举 = 扫目录（按 `started_at` 倒序）；损坏文件跳过并记入告警，绝不 panic。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::storage;

/// 运行记录目录（相对应用配置目录）：`agent/runs`。
pub const RUNS_SUBDIR: &str = "agent";
pub const RUNS_DIR: &str = "runs";
/// 固定服务端搜索工具名（记录用）。
pub const RECORD_SEARCH_TOOL: &str = "web_search_20250305";

/// 停止原因（`state == stopped` 时用于区分）：
/// `manual`（手动停止，本夜不自动重排）/ `disabled`（停用，再启用可重排）/ `completed` / `deleted`。
pub const STOP_MANUAL: &str = "manual";
pub const STOP_DISABLED: &str = "disabled";
pub const STOP_COMPLETED: &str = "completed";
pub const STOP_DELETED: &str = "deleted";

/// 运行状态（字符串落盘）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Pending,
    Running,
    Succeeded,
    Stopped,
    Interrupted,
    Failed,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Pending => "pending",
            RunState::Running => "running",
            RunState::Succeeded => "succeeded",
            RunState::Stopped => "stopped",
            RunState::Interrupted => "interrupted",
            RunState::Failed => "failed",
        }
    }
    pub fn parse(s: &str) -> Option<RunState> {
        Some(match s {
            "pending" => RunState::Pending,
            "running" => RunState::Running,
            "succeeded" => RunState::Succeeded,
            "stopped" => RunState::Stopped,
            "interrupted" => RunState::Interrupted,
            "failed" => RunState::Failed,
            _ => return None,
        })
    }
}

/// 运行统计（无总次数帽；仅用于报告）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RunStats {
    pub chat_calls: u32,
    pub search_calls: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_search_requests: Option<u32>,
    pub bytes_read: u64,
    /// 主循环实际步数（防失控护栏的观测值；**非**调用次数帽、不参与预算）。
    #[serde(default)]
    pub steps: u32,
}

/// 运行启动时的任务快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSnapshot {
    pub title: String,
    pub notes: String,
    pub paths: Vec<String>,
    /// 任务授权链接（只读抓取范围）；旧记录无该键 → 空数组。
    #[serde(default)]
    pub links: Vec<String>,
}

/// 一次运行的事实记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub task_id: String,
    pub list_file: String,
    pub attempt: u32,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    /// 停止原因（`state == stopped` 时）；`manual|disabled|completed|deleted`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub model: String,
    pub search_tool: String,
    pub stats: RunStats,
    pub snapshot: RunSnapshot,
    pub result_dir: String,
    pub partial: bool,
    /// 本次实际抓取过的授权链接（去重、按首次抓取顺序）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub web_reads: Vec<String>,
}

/// 运行记录目录路径。
pub fn runs_dir(base: &Path) -> PathBuf {
    base.join(RUNS_SUBDIR).join(RUNS_DIR)
}

/// 前端展示用运行摘要（供 `agent_get_status` 活动项与 `agent_list_runs`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: String,
    pub task_id: String,
    pub list_file: String,
    pub state: String,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub result_dir: String,
    pub partial: bool,
    pub stats: RunStats,
    pub model: String,
}

impl From<&RunRecord> for RunSummary {
    fn from(rec: &RunRecord) -> Self {
        Self {
            run_id: rec.run_id.clone(),
            task_id: rec.task_id.clone(),
            list_file: rec.list_file.clone(),
            state: rec.state.clone(),
            attempt: rec.attempt,
            error_class: rec.error_class.clone(),
            stop_reason: rec.stop_reason.clone(),
            started_at: rec.started_at.clone(),
            ended_at: rec.ended_at.clone(),
            result_dir: rec.result_dir.clone(),
            partial: rec.partial,
            stats: rec.stats.clone(),
            model: rec.model.clone(),
        }
    }
}

/// 校验 run_id 为 ULID（结果文件名 / 路径片段的唯一来源）。
pub fn valid_run_id(run_id: &str) -> bool {
    ulid::Ulid::from_string(run_id).is_ok()
}

/// 原子写入一条运行记录（temp + rename）。
pub fn write_record_in(base: &Path, rec: &RunRecord) -> Result<(), String> {
    if !valid_run_id(&rec.run_id) {
        return Err(format!("非法的 run_id：{}", rec.run_id));
    }
    let dir = runs_dir(base);
    fs::create_dir_all(&dir).map_err(|e| format!("创建运行记录目录失败：{e}"))?;
    let path = dir.join(format!("{}.json", rec.run_id));
    let json = serde_json::to_vec_pretty(rec).map_err(|e| format!("序列化运行记录失败：{e}"))?;
    storage::atomic_write(&path, &json)
}

/// 扫描全部运行记录（按 `started_at` 倒序）；损坏文件跳过并记入告警。
pub fn list_records_in(base: &Path) -> (Vec<RunRecord>, Vec<String>) {
    let dir = runs_dir(base);
    let mut records: Vec<RunRecord> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return (records, warnings);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        match fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<RunRecord>(&t).ok()) {
            Some(rec) => records.push(rec),
            None => warnings.push(format!("跳过损坏的运行记录：{name}")),
        }
    }
    records.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    (records, warnings)
}

/// 按 run_id 读取一条记录。
pub fn find_record_in(base: &Path, run_id: &str) -> Option<RunRecord> {
    if !valid_run_id(run_id) {
        return None;
    }
    let (records, _) = list_records_in(base);
    records.into_iter().find(|r| r.run_id == run_id)
}

/// 当夜手动停止标记文件（`task_id → 当地日期`）：手动停止后**本夜**不再自动重排。
const MANUAL_STOPS_FILE: &str = "manual_stops.json";

pub fn manual_stops_path(base: &Path) -> PathBuf {
    base.join(RUNS_SUBDIR).join(MANUAL_STOPS_FILE)
}

fn load_manual_stops_in(base: &Path) -> std::collections::HashMap<String, String> {
    let path = manual_stops_path(base);
    fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// 该日期（`YYYY-MM-DD`）仍处于“手动停止”的任务 id 集合。
pub fn manual_stops_on_in(base: &Path, date: &str) -> std::collections::HashSet<String> {
    load_manual_stops_in(base)
        .into_iter()
        .filter(|(_, d)| d == date)
        .map(|(k, _)| k)
        .collect()
}

/// 记下任务在 `date` 当夜被手动停止（持久标记；不影响 attempts / 最新记录语义）。
pub fn mark_manual_stop_in(base: &Path, task_id: &str, date: &str) -> Result<(), String> {
    let mut map = load_manual_stops_in(base);
    map.insert(task_id.to_string(), date.to_string());
    let path = manual_stops_path(base);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
    }
    let json = serde_json::to_vec_pretty(&map).map_err(|e| format!("序列化失败：{e}"))?;
    storage::atomic_write(&path, &json)
}

/// 清除任务的手动停止标记（用户显式 force 立即 / 重跑时解除）。
pub fn clear_manual_stop_in(base: &Path, task_id: &str) -> Result<(), String> {
    let mut map = load_manual_stops_in(base);
    if map.remove(task_id).is_none() {
        return Ok(());
    }
    let path = manual_stops_path(base);
    let json = serde_json::to_vec_pretty(&map).map_err(|e| format!("序列化失败：{e}"))?;
    storage::atomic_write(&path, &json)
}

/// 启动时恢复：`running` → `interrupted`（保留部分成果、不自动续跑）。
///
/// 返回被恢复的 run_id 列表；写回失败的不计入且不影响其它记录。
pub fn recover_interrupted_in(base: &Path, now_local: &str) -> Vec<String> {
    let (records, _) = list_records_in(base);
    let mut recovered = Vec::new();
    for mut rec in records {
        if rec.state != RunState::Running.as_str() {
            continue;
        }
        rec.state = RunState::Interrupted.as_str().to_string();
        rec.partial = true;
        if rec.ended_at.is_none() {
            rec.ended_at = Some(now_local.to_string());
        }
        if write_record_in(base, &rec).is_ok() {
            recovered.push(rec.run_id);
        }
    }
    recovered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(run_id: &str, task_id: &str, state: RunState, started: &str) -> RunRecord {
        RunRecord {
            run_id: run_id.to_string(),
            task_id: task_id.to_string(),
            list_file: "L.yaml".to_string(),
            attempt: 1,
            state: state.as_str().to_string(),
            error_class: None,
            stop_reason: None,
            started_at: started.to_string(),
            ended_at: None,
            model: "deepseek-flash".to_string(),
            search_tool: RECORD_SEARCH_TOOL.to_string(),
            stats: RunStats::default(),
            snapshot: RunSnapshot {
                title: "t".to_string(),
                notes: String::new(),
                paths: vec![],
                links: vec![],
            },
            result_dir: "D:\\r".to_string(),
            partial: false,
            web_reads: vec![],
        }
    }

    fn ulid() -> String {
        ulid::Ulid::generate().to_string()
    }

    #[test]
    fn write_list_and_find_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let id = ulid();
        let rec = sample(&id, "T1", RunState::Succeeded, "2026-10-12T01:00");
        write_record_in(base, &rec).unwrap();

        let (records, warnings) = list_records_in(base);
        assert!(warnings.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].run_id, id);
        assert_eq!(
            find_record_in(base, &id).unwrap().state,
            RunState::Succeeded.as_str()
        );
        // 非法 run_id 不落盘 / 找不到
        assert!(write_record_in(base, &sample("../evil", "T", RunState::Failed, "x")).is_err());
        assert!(find_record_in(base, "../evil").is_none());
    }

    #[test]
    fn web_reads_roundtrip_and_old_records_stay_readable() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let id = ulid();
        let mut rec = sample(&id, "T1", RunState::Succeeded, "2026-10-12T01:00");
        rec.web_reads = vec!["https://a.example/x".to_string(), "file:///D:/x".to_string()];
        rec.snapshot.links = vec!["https://a.example/x".to_string()];
        write_record_in(base, &rec).unwrap();

        let back = find_record_in(base, &id).unwrap();
        assert_eq!(back.web_reads, rec.web_reads);
        assert_eq!(back.snapshot.links, rec.snapshot.links);

        // 旧记录缺少 links / web_reads → 宽容读取为空数组
        let old = ulid();
        let bare = sample(&old, "T1", RunState::Succeeded, "2026-10-12T00:30");
        let mut json = serde_json::to_value(&bare).unwrap();
        json.as_object_mut().unwrap().remove("web_reads");
        json["snapshot"].as_object_mut().unwrap().remove("links");
        fs::create_dir_all(runs_dir(base)).unwrap();
        fs::write(
            runs_dir(base).join(format!("{old}.json")),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        let loaded = find_record_in(base, &old).unwrap();
        assert!(loaded.web_reads.is_empty());
        assert!(loaded.snapshot.links.is_empty());
    }

    #[test]
    fn list_sorts_desc_and_skips_corrupt() {        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let a = ulid();
        let b = ulid();
        write_record_in(base, &sample(&a, "T1", RunState::Succeeded, "2026-10-12T01:00")).unwrap();
        write_record_in(base, &sample(&b, "T1", RunState::Failed, "2026-10-12T02:00")).unwrap();
        // 损坏文件
        fs::write(runs_dir(base).join("broken.json"), "{ not json").unwrap();

        let (records, warnings) = list_records_in(base);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].run_id, b, "较新（02:00）在前");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("broken.json"));
    }

    #[test]
    fn recover_turns_running_into_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let running = ulid();
        let done = ulid();
        write_record_in(base, &sample(&running, "T1", RunState::Running, "2026-10-12T01:00")).unwrap();
        write_record_in(base, &sample(&done, "T1", RunState::Succeeded, "2026-10-12T00:30")).unwrap();

        let recovered = recover_interrupted_in(base, "2026-10-12T09:30");
        assert_eq!(recovered, vec![running.clone()]);

        let rec = find_record_in(base, &running).unwrap();
        assert_eq!(rec.state, RunState::Interrupted.as_str());
        assert!(rec.partial);
        assert_eq!(rec.ended_at.as_deref(), Some("2026-10-12T09:30"));
        // 已成功的记录不动
        assert_eq!(
            find_record_in(base, &done).unwrap().state,
            RunState::Succeeded.as_str()
        );
        // 幂等：再次恢复无 running
        assert!(recover_interrupted_in(base, "2026-10-12T10:00").is_empty());
    }

    #[test]
    fn manual_stop_markers_are_scoped_by_date() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        assert!(manual_stops_on_in(base, "2026-10-12").is_empty());
        mark_manual_stop_in(base, "T1", "2026-10-12").unwrap();
        mark_manual_stop_in(base, "T2", "2026-10-12").unwrap();

        let today = manual_stops_on_in(base, "2026-10-12");
        assert_eq!(today.len(), 2);
        assert!(today.contains("T1") && today.contains("T2"));
        // 新夜自动解除
        assert!(manual_stops_on_in(base, "2026-10-13").is_empty());

        // 显式解除
        clear_manual_stop_in(base, "T1").unwrap();
        let today = manual_stops_on_in(base, "2026-10-12");
        assert!(!today.contains("T1") && today.contains("T2"));

        // 损坏文件安全回退为空
        fs::write(manual_stops_path(base), "{ not json").unwrap();
        assert!(manual_stops_on_in(base, "2026-10-12").is_empty());
    }
}
