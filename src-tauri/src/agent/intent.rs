//! 任务 Agent 意图：任务 YAML 内 `agent: { enabled, paths, links }`（按 id 原子补丁，保留未知字段）。
//!
//! 关联 ≠ 授权：此处只存原始绝对路径；实际读取在运行时经沙箱校验（Phase 2）。
//! `links` 供人工点击打开，同时作为只读抓取工具 `url_read` 的授权范围；抓取内容作不可信数据。
//! `enabled: true` 的前置（本机执行授权 / 密钥 / 结果根）由命令层校验。

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use yaml_serde::{Mapping, Value};

use super::sandbox;
use crate::storage;

/// 任务上的 Agent 意图视图（随 `TaskView` 返回）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskAgent {
    pub enabled: bool,
    pub paths: Vec<String>,
    /// 人工点击打开的链接；亦为 `url_read` 的授权范围。旧数据无该键 → 空数组。
    #[serde(default)]
    pub links: Vec<String>,
}

/// 从任务映射解析 `agent` 字段；缺失 / 非法不 panic，宽返回 `None` 或默认值。
pub fn parse_task_agent(m: &Mapping) -> Option<TaskAgent> {
    let agent = m.get("agent")?.as_mapping()?;
    let enabled = agent
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let paths = agent
        .get("paths")
        .and_then(Value::as_sequence)
        .map(|seq| {
            seq.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let links = agent
        .get("links")
        .and_then(Value::as_sequence)
        .map(|seq| {
            seq.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some(TaskAgent {
        enabled,
        paths,
        links,
    })
}

/// 链接形状校验（写入前）：trim 后长度 / 空白 / 协议前缀 / 去重保序。
///
/// 仅用于人工点击打开与 `url_read` 授权范围形状校验；返回 trim 后的值。
pub fn validate_links(links: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::with_capacity(links.len());
    let mut seen: HashSet<String> = HashSet::new();
    for raw in links {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("链接不能为空".to_string());
        }
        if trimmed.chars().count() > 2048 {
            return Err("链接过长（上限 2048 字符）".to_string());
        }
        if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("链接不能包含空白或控制字符".to_string());
        }
        let lower = trimmed.to_ascii_lowercase();
        let allowed = ["http://", "https://", "file://"]
            .iter()
            .any(|prefix| lower.starts_with(prefix));
        if !allowed {
            return Err(format!(
                "链接必须以 http://、https:// 或 file:// 开头：{trimmed}"
            ));
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }
    Ok(out)
}

/// 按 `task_id` 补丁任务的 `agent.enabled` / `agent.paths` / `agent.links`，保留其它 / 未知字段并原子写。
///
/// 空白路径报错；`enabled == true` 时要求每个路径为绝对路径（关联≠授权，此处只做形状校验）。
/// `links` 只做形状校验（见 [`validate_links`]），不参与沙箱。
pub fn set_task_intent_in(
    base: &Path,
    list_file: &str,
    task_id: &str,
    enabled: bool,
    paths: &[String],
    links: &[String],
) -> Result<(), String> {
    for p in paths {
        if p.trim().is_empty() {
            return Err("关联路径不能为空".to_string());
        }
        // 存前对每个路径套源根语义护栏（与结果根同等强度）；P2 runner 启动处理已存根时
        // 应对每个 `agent.paths` 再调用同一 `sandbox::guard_source_root`（外部 YAML 可绕过命令）。
        // 全有或全无：任一路径不合格则整批不写；错误点名该路径便于用户定位。
        sandbox::guard_source_root(p).map_err(|e| format!("路径「{p}」不可用：{e}"))?;
    }
    let links = validate_links(links)?;

    let path = storage::list_path(base, list_file)?;
    let mut value = storage::read_list_value(&path, list_file)?;
    let map = storage::find_task_mut(&mut value, list_file, task_id)?;

    let agent = match map.get_mut("agent") {
        Some(existing) => existing
            .as_mapping_mut()
            .ok_or_else(|| format!("任务 {task_id} 的 agent 不是对象"))?,
        None => {
            map.insert(storage::key("agent"), Value::Mapping(Mapping::new()));
            map.get_mut("agent")
                .and_then(Value::as_mapping_mut)
                .expect("刚插入的 agent 应为对象")
        }
    };
    agent.insert(storage::key("enabled"), Value::Bool(enabled));
    agent.insert(
        storage::key("paths"),
        Value::Sequence(paths.iter().map(|p| Value::String(p.clone())).collect()),
    );
    agent.insert(
        storage::key("links"),
        Value::Sequence(links.iter().map(|l| Value::String(l.clone())).collect()),
    );

    storage::write_list(&path, &value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::LISTS_DIR;
    use std::fs;

    fn write_list(base: &Path, file: &str, text: &str) {
        let lists = base.join(LISTS_DIR);
        fs::create_dir_all(&lists).unwrap();
        fs::write(lists.join(file), text).unwrap();
    }

    fn read(base: &Path, file: &str) -> Value {
        yaml_serde::from_str(&fs::read_to_string(base.join(LISTS_DIR).join(file)).unwrap()).unwrap()
    }

    #[test]
    fn sets_enabled_and_paths_preserving_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\nx-top: keep\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    x-task: keep2\n    subtasks:\n      - id: S1\n        title: 甲\n        status: open\n  - id: T2\n    title: 二\n    status: open\n",
        );

        set_task_intent_in(
            base,
            "L.yaml",
            "T1",
            true,
            &["D:\\docs\\a.txt".to_string(), "D:\\proj".to_string()],
            &[],
        )
        .unwrap();

        let value = read(base, "L.yaml");
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let tasks = value.get("tasks").and_then(Value::as_sequence).unwrap();
        let t1 = tasks
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("T1"))
            .unwrap();
        let agent = t1.get("agent").and_then(Value::as_mapping).unwrap();
        assert_eq!(agent.get("enabled").and_then(Value::as_bool), Some(true));
        assert_eq!(
            agent
                .get("paths")
                .and_then(Value::as_sequence)
                .map(|s| s.len()),
            Some(2)
        );
        // 子任务与其它字段保留
        assert_eq!(t1.get("x-task").and_then(Value::as_str), Some("keep2"));
        assert_eq!(
            t1.get("subtasks").and_then(Value::as_sequence).map(|s| s.len()),
            Some(1)
        );
        // 其它任务不被动
        let t2 = tasks
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("T2"))
            .unwrap();
        assert!(t2.get("agent").is_none());
    }

    #[test]
    fn preserves_unknown_fields_inside_agent() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    agent:\n      enabled: false\n      x-extra: keep\n",
        );

        set_task_intent_in(base, "L.yaml", "T1", false, &["D:\\p".to_string()], &[]).unwrap();

        let value = read(base, "L.yaml");
        let agent = value.get("tasks").and_then(Value::as_sequence).unwrap()[0]
            .get("agent")
            .and_then(Value::as_mapping)
            .unwrap()
            .clone();
        assert_eq!(agent.get("x-extra").and_then(Value::as_str), Some("keep"));
        assert_eq!(agent.get("enabled").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn sets_links_trimmed_deduped_and_rejects_invalid_without_write() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n",
        );

        set_task_intent_in(
            base,
            "L.yaml",
            "T1",
            false,
            &[],
            &[
                " https://a.example/x ".to_string(),
                "HTTP://B.example".to_string(),
                "https://a.example/x".to_string(),
                "file:///D:/x".to_string(),
            ],
        )
        .unwrap();

        let value = read(base, "L.yaml");
        let agent = value.get("tasks").and_then(Value::as_sequence).unwrap()[0]
            .get("agent")
            .and_then(Value::as_mapping)
            .unwrap()
            .clone();
        let links: Vec<&str> = agent
            .get("links")
            .and_then(Value::as_sequence)
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(
            links,
            vec!["https://a.example/x", "HTTP://B.example", "file:///D:/x"],
            "trim + 去重保序"
        );

        let before = fs::read_to_string(base.join(LISTS_DIR).join("L.yaml")).unwrap();
        let too_long = format!("https://{}", "a".repeat(2048));
        for bad in [
            "javascript:alert(1)".to_string(),
            "ftp://x".to_string(),
            "D:\\a.txt".to_string(),
            "https://a b".to_string(),
            "   ".to_string(),
            too_long,
        ] {
            assert!(
                set_task_intent_in(base, "L.yaml", "T1", false, &[], std::slice::from_ref(&bad))
                    .is_err(),
                "非法链接应被拒：{bad:?}"
            );
        }
        assert_eq!(
            fs::read_to_string(base.join(LISTS_DIR).join("L.yaml")).unwrap(),
            before,
            "拒绝后不应写盘"
        );
    }

    #[test]
    fn unsafe_or_relative_paths_rejected_regardless_of_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n",
        );

        // 相对路径一律拒绝（架构 §5：只存原始绝对路径）
        assert!(set_task_intent_in(base, "L.yaml", "T1", false, &["rel\\a.txt".to_string()], &[]).is_err());
        assert!(set_task_intent_in(base, "L.yaml", "T1", true, &["rel\\a.txt".to_string()], &[]).is_err());
        // 空白路径报错
        assert!(set_task_intent_in(base, "L.yaml", "T1", false, &["   ".to_string()], &[]).is_err());
        // 未知任务报错
        assert!(set_task_intent_in(base, "L.yaml", "NOPE", false, &[], &[]).is_err());
        // 合法绝对路径可存（停用态）
        set_task_intent_in(base, "L.yaml", "T1", false, &["D:\\p".to_string()], &[]).unwrap();
    }

    #[test]
    fn dangerous_authorization_roots_rejected_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n",
        );

        // P1-4：盘符根 / 设备路径 / UNC 共享根 / 含 .. 的形状一律拒绝
        for p in [
            "C:\\",
            "\\\\?\\C:\\a",
            "\\\\server\\share",
            "\\\\wsl.localhost\\Ubuntu",
            "C:\\a\\..\\b",
            "//server/share/a",
        ] {
            assert!(
                set_task_intent_in(base, "L.yaml", "T1", true, &[p.to_string()], &[]).is_err(),
                "危险授权根应被拒：{p}"
            );
            assert!(
                set_task_intent_in(base, "L.yaml", "T1", false, &[p.to_string()], &[]).is_err(),
                "停用态也不应存危险根：{p}"
            );
        }
        // 全部被拒 → 未写盘
        let value = read(base, "L.yaml");
        let t1 = &value.get("tasks").and_then(Value::as_sequence).unwrap()[0];
        assert!(t1.get("agent").is_none(), "拒绝后不应写入 agent");

        // UNC 子路径是合法授权根（共享根之外的任意层级）
        for p in [
            "\\\\server\\share\\proj",
            "\\\\wsl.localhost\\Ubuntu\\home\\me",
            "\\\\wsl$\\Ubuntu\\home\\me\\notes.md",
        ] {
            set_task_intent_in(base, "L.yaml", "T1", true, &[p.to_string()], &[])
                .unwrap_or_else(|e| panic!("UNC 子路径应可授权：{p} / {e}"));
        }
    }

    #[test]
    fn rejects_non_mapping_agent_without_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    agent: scalar\n",
        );
        let before = fs::read_to_string(base.join(LISTS_DIR).join("L.yaml")).unwrap();
        assert!(set_task_intent_in(base, "L.yaml", "T1", false, &[], &[]).is_err());
        assert_eq!(
            fs::read_to_string(base.join(LISTS_DIR).join("L.yaml")).unwrap(),
            before
        );
    }

    #[test]
    fn parse_task_agent_is_tolerant() {
        let load = |text: &str| -> Option<TaskAgent> {
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path();
            write_list(base, "L.yaml", text);
            let value = read(base, "L.yaml");
            let task = value
                .get("tasks")
                .and_then(Value::as_sequence)
                .and_then(|s| s.first())?;
            parse_task_agent(task.as_mapping()?)
        };

        assert!(load("schema: 1\ntasks:\n  - id: T1\n    title: x\n").is_none());
        let a = load("schema: 1\ntasks:\n  - id: T1\n    agent:\n      enabled: true\n").unwrap();
        assert!(a.enabled);
        assert!(a.paths.is_empty());
        assert!(a.links.is_empty(), "旧数据无 links → 空数组");
        // enabled 非布尔 / paths 非数组 → 宽容
        let b = load("schema: 1\ntasks:\n  - id: T1\n    agent:\n      enabled: nope\n      paths: 5\n")
            .unwrap();
        assert!(!b.enabled);
        assert!(b.paths.is_empty());
        // links：非字符串丢弃、非数组宽容
        let c = load(
            "schema: 1\ntasks:\n  - id: T1\n    agent:\n      links:\n        - https://a.example\n        - 5\n        - file:///D:/x\n",
        )
        .unwrap();
        assert_eq!(c.links, vec!["https://a.example", "file:///D:/x"]);
        assert!(load("schema: 1\ntasks:\n  - id: T1\n    agent:\n      links: nope\n")
            .unwrap()
            .links
            .is_empty());
        // agent 非对象 → None
        assert!(load("schema: 1\ntasks:\n  - id: T1\n    agent: scalar\n").is_none());
    }

    // ---- WSL UNC 路径：接受 / 全有或全无 ----

    fn agent_paths(base: &Path, file: &str) -> Vec<String> {
        let value = read(base, file);
        value
            .get("tasks")
            .and_then(Value::as_sequence)
            .and_then(|s| s.first())
            .and_then(|t| t.get("agent"))
            .and_then(Value::as_mapping)
            .and_then(|a| a.get("paths"))
            .and_then(Value::as_sequence)
            .map(|seq| {
                seq.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn wsl_unc_path_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n",
        );

        let wsl = "\\\\wsl.localhost\\Ubuntu\\home\\me\\notes.md".to_string();
        set_task_intent_in(base, "L.yaml", "T1", false, std::slice::from_ref(&wsl), &[])
            .expect("WSL UNC 路径应被接受（用户从资源管理器选中）");

        assert_eq!(
            agent_paths(base, "L.yaml"),
            vec![wsl],
            "agent.paths 应包含所选 WSL 路径（原样写入）"
        );
    }

    #[test]
    fn wsl_dollar_unc_path_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n",
        );

        let wsl = "\\\\wsl$\\Ubuntu\\home\\me\\notes.md".to_string();
        set_task_intent_in(base, "L.yaml", "T1", false, std::slice::from_ref(&wsl), &[])
            .expect("wsl$ UNC 路径应被接受（用户从资源管理器选中）");

        assert_eq!(
            agent_paths(base, "L.yaml"),
            vec![wsl],
            "agent.paths 应包含所选 wsl$ 路径"
        );
    }

    #[test]
    fn unc_paths_written_verbatim_including_dir() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n",
        );

        // 目录、文件、网络共享 → 一律原样写入，不做规范化
        let paths = vec![
            "\\\\server\\share\\proj".to_string(),
            "\\\\server\\share\\proj\\a b.txt".to_string(),
            "\\\\wsl.localhost\\Ubuntu\\home\\me".to_string(),
            "D:\\docs\\a.txt".to_string(),
        ];
        set_task_intent_in(base, "L.yaml", "T1", true, &paths, &[]).unwrap();
        assert_eq!(agent_paths(base, "L.yaml"), paths);
    }

    #[test]
    fn one_bad_path_rejects_whole_batch_and_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n",
        );

        let good = "C:\\Users\\me\\ok.txt".to_string();
        let bad = "C:\\Users\\me\\dir.\\b.txt".to_string();
        let err = set_task_intent_in(
            base,
            "L.yaml",
            "T1",
            true,
            &[good.clone(), bad.clone()],
            &[],
        )
        .expect_err("含一条不合格路径时整批应被拒");

        assert!(
            err.contains(&bad),
            "错误必须点名 offending 路径，实际：{err}"
        );
        assert!(
            !err.contains(&good),
            "不应把好路径也算作问题：{err}"
        );
        assert!(agent_paths(base, "L.yaml").is_empty(), "整批不写盘");
    }

    #[test]
    fn wsl_internal_path_rejected_with_hint() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n",
        );

        for p in ["/home/me/notes.md", "/mnt/c/Users/me/x.txt"] {
            let err = set_task_intent_in(base, "L.yaml", "T1", false, &[p.to_string()], &[])
                .expect_err("WSL 内部路径应被拒");
            assert!(err.contains(p), "应点名路径：{err}");
            assert!(err.contains("WSL"), "应提示 WSL 内部路径不可访问：{err}");
        }
        assert!(agent_paths(base, "L.yaml").is_empty());
    }

    #[test]
    fn sensitive_unc_path_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n",
        );

        let p = "\\\\server\\share\\.ssh\\id_rsa";
        let err = set_task_intent_in(base, "L.yaml", "T1", true, &[p.to_string()], &[])
            .expect_err("UNC 下的敏感组件同样应被拒");
        assert!(err.contains(p), "应点名路径：{err}");
    }
}
