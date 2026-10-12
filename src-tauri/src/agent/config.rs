//! Agent 非敏感本机配置：`settings.yaml` 的 `agent:` 映射（Value 级补丁，保留未知字段）。
//!
//! 只处理已知字段 `model` / `result_root` / `execution_enabled` / `max_uses`；
//! 缺失 / 非法值安全回退默认；`theme`、`app_bar` 及其它未知字段一律不动。

use std::fs;
use std::path::Path;

use serde::Serialize;
use yaml_serde::{Mapping, Value};

use super::{DEFAULT_MAX_USES, DEFAULT_MODEL};
use crate::storage;

/// 已加载的 Agent 配置（不含密钥）。
#[derive(Debug, Clone, PartialEq)]
pub struct AgentConfig {
    pub model: String,
    pub result_root: Option<String>,
    pub execution_enabled: bool,
    pub max_uses: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            result_root: None,
            execution_enabled: false,
            max_uses: DEFAULT_MAX_USES,
        }
    }
}

/// 返回前端的视图（snake_case 字段 + 密钥是否存在）。**不含密钥值**。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentConfigView {
    pub model: String,
    pub result_root: Option<String>,
    pub execution_enabled: bool,
    pub max_uses: u32,
    pub key_present: bool,
}

impl AgentConfig {
    pub fn view(&self, key_present: bool) -> AgentConfigView {
        AgentConfigView {
            model: self.model.clone(),
            result_root: self.result_root.clone(),
            execution_enabled: self.execution_enabled,
            max_uses: self.max_uses,
            key_present,
        }
    }
}

/// 读取 `settings.yaml` 的 `agent` 配置；文件缺失 / 解析失败 / 非法值一律回退默认，
/// **不创建文件、不 panic**。
pub fn load_agent_config_from(base: &Path) -> AgentConfig {
    let path = base.join(storage::SETTINGS_FILE);
    let value = match fs::read_to_string(&path) {
        Ok(text) => match yaml_serde::from_str::<Value>(&text) {
            Ok(value) => value,
            Err(_) => return AgentConfig::default(),
        },
        Err(_) => return AgentConfig::default(),
    };
    parse_agent_config(&value)
}

fn parse_agent_config(value: &Value) -> AgentConfig {
    let defaults = AgentConfig::default();
    let Some(agent) = value.get("agent").and_then(Value::as_mapping) else {
        return defaults;
    };

    let model = agent
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or(defaults.model);

    let result_root = agent
        .get("result_root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let execution_enabled = agent
        .get("execution_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let max_uses = agent
        .get("max_uses")
        .and_then(Value::as_i64)
        .filter(|n| *n >= 1)
        .map(|n| n as u32)
        .unwrap_or(defaults.max_uses);

    AgentConfig {
        model,
        result_root,
        execution_enabled,
        max_uses,
    }
}

/// 待补丁的已知字段（由前端 `config` 对象解析；缺省字段不动）。
#[derive(Debug, Default)]
struct ConfigPatch {
    model: Option<String>,
    /// 外层 `None` = 未提供；`Some(None)` = 显式清空；`Some(Some(p))` = 设置路径。
    result_root: Option<Option<String>>,
    execution_enabled: Option<bool>,
    max_uses: Option<u32>,
}

fn parse_patch(config: &serde_json::Value) -> Result<ConfigPatch, String> {
    let obj = config
        .as_object()
        .ok_or_else(|| "配置必须为对象".to_string())?;

    let model = match obj.get("model") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                return Err("模型名不能为空".to_string());
            }
            Some(t.to_string())
        }
        Some(_) => return Err("模型名格式无效".to_string()),
    };

    let result_root = match obj.get("result_root") {
        None => None,
        Some(serde_json::Value::Null) => Some(None),
        Some(serde_json::Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                Some(None)
            } else {
                Some(Some(t.to_string()))
            }
        }
        Some(_) => return Err("结果目录格式无效".to_string()),
    };

    let execution_enabled = match obj.get("execution_enabled") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Bool(b)) => Some(*b),
        Some(_) => return Err("执行开关格式无效".to_string()),
    };

    let max_uses = match obj.get("max_uses") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Number(n)) => {
            let v = n
                .as_u64()
                .filter(|v| *v >= 1)
                .ok_or_else(|| "搜索上限必须为正整数".to_string())?;
            Some(v as u32)
        }
        Some(_) => return Err("搜索上限必须为正整数".to_string()),
    };

    Ok(ConfigPatch {
        model,
        result_root,
        execution_enabled,
        max_uses,
    })
}

/// 以 Value 级补丁写入 `settings.yaml` 的 `agent` **已知字段**，保留其它 / 未知字段。
///
/// `execution_enabled` 由 false → true 当且仅当 `consent_cloud == true`，否则报错不写。
pub fn save_agent_config_in(
    base: &Path,
    config: &serde_json::Value,
    consent_cloud: bool,
) -> Result<(), String> {
    let patch = parse_patch(config)?;

    fs::create_dir_all(base).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let path = base.join(storage::SETTINGS_FILE);
    let mut value = if path.is_file() {
        let text = fs::read_to_string(&path).map_err(|e| format!("读取设置文件失败：{e}"))?;
        yaml_serde::from_str(&text).map_err(|e| format!("解析设置文件失败：{e}"))?
    } else {
        Value::Mapping(Mapping::new())
    };

    let current = parse_agent_config(&value);
    if patch.execution_enabled == Some(true) && !current.execution_enabled && !consent_cloud {
        return Err("启用本机执行需显式云端同意".to_string());
    }

    let root = value
        .as_mapping_mut()
        .ok_or_else(|| "设置文件格式无效：期望对象".to_string())?;
    let agent = match root.get_mut("agent") {
        Some(existing) => existing
            .as_mapping_mut()
            .ok_or_else(|| "设置文件中的 agent 不是对象，无法写入".to_string())?,
        None => {
            root.insert(storage::key("agent"), Value::Mapping(Mapping::new()));
            root.get_mut("agent")
                .and_then(Value::as_mapping_mut)
                .expect("刚插入的 agent 应为对象")
        }
    };

    if let Some(model) = patch.model {
        agent.insert(storage::key("model"), Value::String(model));
    }
    if let Some(root_path) = patch.result_root {
        match root_path {
            Some(p) => {
                agent.insert(storage::key("result_root"), Value::String(p));
            }
            None => {
                agent.insert(storage::key("result_root"), Value::Null);
            }
        }
    }
    if let Some(enabled) = patch.execution_enabled {
        agent.insert(storage::key("execution_enabled"), Value::Bool(enabled));
    }
    if let Some(max_uses) = patch.max_uses {
        agent.insert(
            storage::key("max_uses"),
            Value::Number((max_uses as i64).into()),
        );
    }

    storage::write_list(&path, &value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_settings(base: &Path, text: &str) {
        fs::write(base.join(storage::SETTINGS_FILE), text).unwrap();
    }

    #[test]
    fn load_defaults_when_missing_or_bad() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let cfg = load_agent_config_from(base);
        assert_eq!(cfg, AgentConfig::default());
        assert!(!base.join(storage::SETTINGS_FILE).exists());

        // 坏文件 / 非对象 / agent 非对象 → 默认
        for text in ["agent: [unclosed\n", "- a\n", "agent: notamap\n"] {
            write_settings(base, text);
            assert_eq!(load_agent_config_from(base), AgentConfig::default());
        }
    }

    #[test]
    fn load_parses_known_fields_and_ignores_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_settings(
            base,
            "theme: dark\nagent:\n  model: custom\n  result_root: \"D:\\\\out\"\n  execution_enabled: true\n  max_uses: 9\n  x-unknown: keep\n",
        );
        let cfg = load_agent_config_from(base);
        assert_eq!(cfg.model, "custom");
        assert_eq!(cfg.result_root.as_deref(), Some("D:\\out"));
        assert!(cfg.execution_enabled);
        assert_eq!(cfg.max_uses, 9);

        // 非法值逐一回退
        write_settings(
            base,
            "agent:\n  model: '   '\n  result_root: 42\n  execution_enabled: nope\n  max_uses: 0\n",
        );
        let cfg = load_agent_config_from(base);
        assert_eq!(cfg.model, DEFAULT_MODEL);
        assert_eq!(cfg.result_root, None);
        assert!(!cfg.execution_enabled);
        assert_eq!(cfg.max_uses, DEFAULT_MAX_USES);
    }

    #[test]
    fn save_patches_known_fields_preserving_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_settings(
            base,
            "theme: dark\nx-top: keep\napp_bar:\n  order: [refresh]\nagent:\n  x-known-unknown: keep2\n  model: old\n",
        );

        save_agent_config_in(
            base,
            &json!({ "model": "m1", "result_root": "D:\\res", "max_uses": 3 }),
            false,
        )
        .unwrap();

        let cfg = load_agent_config_from(base);
        assert_eq!(cfg.model, "m1");
        assert_eq!(cfg.result_root.as_deref(), Some("D:\\res"));
        assert!(!cfg.execution_enabled, "未提供则不动（默认 false）");
        assert_eq!(cfg.max_uses, 3);

        let value: Value =
            yaml_serde::from_str(&fs::read_to_string(base.join(storage::SETTINGS_FILE)).unwrap())
                .unwrap();
        assert_eq!(value.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        assert_eq!(
            value
                .get("app_bar")
                .and_then(Value::as_mapping)
                .and_then(|m| m.get("order"))
                .and_then(Value::as_sequence)
                .map(|s| s.len()),
            Some(1)
        );
        assert_eq!(
            value
                .get("agent")
                .and_then(Value::as_mapping)
                .and_then(|m| m.get("x-known-unknown"))
                .and_then(Value::as_str),
            Some("keep2"),
            "agent 内未知字段应保留"
        );
    }

    #[test]
    fn save_requires_consent_for_enable_transition() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        // false→true 无同意 → 报错且不写
        let err = save_agent_config_in(base, &json!({ "execution_enabled": true }), false).unwrap_err();
        assert!(err.contains("同意"), "错误应提示需同意：{err}");
        assert!(!load_agent_config_from(base).execution_enabled);

        // 带同意 → 成功
        save_agent_config_in(base, &json!({ "execution_enabled": true }), true).unwrap();
        assert!(load_agent_config_from(base).execution_enabled);

        // 已 true → 再次 true 无需同意
        save_agent_config_in(base, &json!({ "execution_enabled": true }), false).unwrap();
        assert!(load_agent_config_from(base).execution_enabled);

        // true→false 无需同意
        save_agent_config_in(base, &json!({ "execution_enabled": false }), false).unwrap();
        assert!(!load_agent_config_from(base).execution_enabled);
    }

    #[test]
    fn save_rejects_invalid_input_and_malformed_agent() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        assert!(save_agent_config_in(base, &json!("x"), false).is_err());
        assert!(save_agent_config_in(base, &json!({ "model": "   " }), false).is_err());
        assert!(save_agent_config_in(base, &json!({ "max_uses": 0 }), false).is_err());
        assert!(save_agent_config_in(base, &json!({ "execution_enabled": "yes" }), false).is_err());
        assert!(save_agent_config_in(base, &json!({ "result_root": 5 }), false).is_err());

        // agent 为非对象 → 报错且不覆盖
        write_settings(base, "agent: scalar\n");
        let before = fs::read_to_string(base.join(storage::SETTINGS_FILE)).unwrap();
        assert!(save_agent_config_in(base, &json!({ "model": "m" }), false).is_err());
        assert_eq!(
            fs::read_to_string(base.join(storage::SETTINGS_FILE)).unwrap(),
            before
        );

        // 根非对象 → 报错
        write_settings(base, "- a\n- b\n");
        assert!(save_agent_config_in(base, &json!({ "model": "m" }), false).is_err());
    }

    #[test]
    fn save_can_clear_result_root_and_create_file() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        save_agent_config_in(
            base,
            &json!({ "model": "m", "result_root": "D:\\r" }),
            false,
        )
        .unwrap();
        assert_eq!(load_agent_config_from(base).result_root.as_deref(), Some("D:\\r"));

        save_agent_config_in(base, &json!({ "result_root": null }), false).unwrap();
        assert_eq!(load_agent_config_from(base).result_root, None);
    }
}
