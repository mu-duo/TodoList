//! Agent 非敏感本机配置：`settings.yaml` 的 `agent:` 映射（Value 级补丁，保留未知字段）。
//!
//! 只处理已知字段 `provider` / `model` / `result_root` / `execution_enabled` / `max_uses` /
//! `custom` / `model_by_provider`；缺失 / 非法值安全回退默认；`provider` 未知值**明确报错**
//! （fail closed）；`theme`、`app_bar` 及其它未知字段一律不动。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Serialize;
use yaml_serde::{Mapping, Value};

use super::{presets, DEFAULT_MAX_USES, DEFAULT_MODEL};
use crate::storage;

/// 自定义端点（settings `agent.custom`）；`protocol` 仅 `"anthropic"` / `"openai"`。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CustomEndpoint {
    pub protocol: String,
    pub base_url: String,
}

/// 已加载的 Agent 配置（不含密钥）。
#[derive(Debug, Clone, PartialEq)]
pub struct AgentConfig {
    /// 当前 provider id（写入 `settings.yaml` 的 `agent.provider`）。
    pub provider: String,
    pub model: String,
    pub result_root: Option<String>,
    pub execution_enabled: bool,
    pub max_uses: u32,
    /// 自定义端点（仅 provider=`custom` 时生效；其余 provider 下惰性保留）。
    pub custom: Option<CustomEndpoint>,
    /// 各 provider 上次使用的模型（provider id → model）。
    pub model_by_provider: BTreeMap<String, String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            provider: presets::DEFAULT_PROVIDER_ID.to_string(),
            model: DEFAULT_MODEL.to_string(),
            result_root: None,
            execution_enabled: false,
            max_uses: DEFAULT_MAX_USES,
            custom: None,
            model_by_provider: BTreeMap::new(),
        }
    }
}

/// 返回前端的视图（snake_case 字段 + 密钥是否存在）。**不含密钥值**。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentConfigView {
    pub provider: String,
    pub model: String,
    pub result_root: Option<String>,
    pub execution_enabled: bool,
    pub max_uses: u32,
    pub key_present: bool,
    pub custom: Option<CustomEndpoint>,
    pub model_by_provider: BTreeMap<String, String>,
}

/// 解析协议字符串（`"anthropic"` / `"openai"`）；其余 → 明确报错（fail closed）。
pub fn parse_protocol_str(s: &str) -> Result<presets::Protocol, String> {
    match s.trim() {
        "anthropic" => Ok(presets::Protocol::AnthropicCompat),
        "openai" => Ok(presets::Protocol::OpenAiCompat),
        other => Err(format!("未知的协议：{other}")),
    }
}

impl AgentConfig {
    pub fn view(&self, key_present: bool) -> AgentConfigView {
        AgentConfigView {
            provider: self.provider.clone(),
            model: self.model.clone(),
            result_root: self.result_root.clone(),
            execution_enabled: self.execution_enabled,
            max_uses: self.max_uses,
            key_present,
            custom: self.custom.clone(),
            model_by_provider: self.model_by_provider.clone(),
        }
    }

    /// 解析当前 provider 预设；未知 → 明确报错（fail closed）。`custom` 不属预设，会报错。
    pub fn provider_preset(&self) -> Result<&'static presets::ProviderPreset, String> {
        presets::resolve(Some(&self.provider))
    }

    /// 校验 provider 值合法（预设或 `custom`）；未知 → 报错（fail closed）。
    pub fn validate_provider(&self) -> Result<(), String> {
        if self.provider == presets::CUSTOM_PROVIDER_ID {
            Ok(())
        } else {
            self.provider_preset().map(|_| ())
        }
    }

    /// 解析当前 provider 的已解析规格：预设 → `resolve_provider`；`custom` → `resolve_custom`。
    /// 未知 provider / 非法 custom → 明确报错（fail closed）。
    pub fn resolved(&self) -> Result<presets::ResolvedProvider, String> {
        if self.provider == presets::CUSTOM_PROVIDER_ID {
            let custom = self
                .custom
                .as_ref()
                .ok_or_else(|| "自定义端点未配置".to_string())?;
            let protocol = parse_protocol_str(&custom.protocol)?;
            presets::resolve_custom(protocol, &custom.base_url)
        } else {
            let preset = self.provider_preset()?;
            presets::resolve_provider(preset, &self.model)
        }
    }
}

/// 读取 `settings.yaml` 的 `agent` 配置；文件缺失 / 解析失败 / 非法值一律回退默认，
/// **不创建文件、不 panic**。读取前剥离前导 BOM（[`crate::storage::strip_bom`]）。
pub fn load_agent_config_from(base: &Path) -> AgentConfig {
    let path = base.join(storage::SETTINGS_FILE);
    let value = match fs::read_to_string(&path) {
        Ok(text) => match yaml_serde::from_str::<Value>(storage::strip_bom(&text)) {
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

    let provider = agent
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or(defaults.provider);

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

    // 自定义端点（惰性解析：结构不完整 → None，不报错；值校验在生效 / 保存时进行）。
    let custom = agent.get("custom").and_then(parse_custom_endpoint);

    // 各 provider 上次模型（宽容解析：跳过非法项）。
    let model_by_provider = agent
        .get("model_by_provider")
        .map(parse_model_map)
        .unwrap_or_default();

    AgentConfig {
        provider,
        model,
        result_root,
        execution_enabled,
        max_uses,
        custom,
        model_by_provider,
    }
}

/// 解析 `agent.custom` 映射（**宽容**：非映射 / 缺字段 → `None`，不报错）。
fn parse_custom_endpoint(value: &Value) -> Option<CustomEndpoint> {
    let m = value.as_mapping()?;
    let protocol = m
        .get("protocol")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let base_url = m
        .get("base_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    Some(CustomEndpoint {
        protocol: protocol.to_string(),
        base_url: base_url.to_string(),
    })
}

/// 解析 `agent.model_by_provider`（**宽容**：跳过非字符串 / 空值项）。
fn parse_model_map(value: &Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(m) = value.as_mapping() {
        for (k, v) in m {
            if let (Some(k), Some(v)) = (k.as_str(), v.as_str()) {
                let (k, v) = (k.trim(), v.trim());
                if !k.is_empty() && !v.is_empty() {
                    out.insert(k.to_string(), v.to_string());
                }
            }
        }
    }
    out
}

/// 自定义端点补丁：设置（结构已校验，值校验在保存时按生效 provider 进行）或清空。
#[derive(Debug)]
enum CustomPatch {
    Set(CustomEndpoint),
    Clear,
}

/// 待补丁的已知字段（由前端 `config` 对象解析；缺省字段不动）。
#[derive(Debug, Default)]
struct ConfigPatch {
    provider: Option<String>,
    model: Option<String>,
    /// 外层 `None` = 未提供；`Some(None)` = 显式清空；`Some(Some(p))` = 设置路径。
    result_root: Option<Option<String>>,
    execution_enabled: Option<bool>,
    max_uses: Option<u32>,
    custom: Option<CustomPatch>,
    model_by_provider: Option<BTreeMap<String, String>>,
}

fn parse_patch(config: &serde_json::Value) -> Result<ConfigPatch, String> {
    let obj = config
        .as_object()
        .ok_or_else(|| "配置必须为对象".to_string())?;

    let provider = match obj.get("provider") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                // 空 = 回默认
                Some(presets::DEFAULT_PROVIDER_ID.to_string())
            } else {
                if t != presets::CUSTOM_PROVIDER_ID {
                    presets::find(t).ok_or_else(|| format!("未知的 provider：{t}"))?;
                }
                Some(t.to_string())
            }
        }
        Some(_) => return Err("provider 格式无效".to_string()),
    };

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

    // 自定义端点（结构校验；值校验在保存时按生效 provider 进行）。
    let custom = match obj.get("custom") {
        None => None,
        Some(serde_json::Value::Null) => Some(CustomPatch::Clear),
        Some(serde_json::Value::Object(o)) => {
            let protocol = o
                .get("protocol")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "custom.protocol 不能为空".to_string())?;
            let base_url = o
                .get("base_url")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "custom.base_url 不能为空".to_string())?;
            Some(CustomPatch::Set(CustomEndpoint {
                protocol: protocol.to_string(),
                base_url: base_url.to_string(),
            }))
        }
        Some(_) => return Err("custom 格式无效".to_string()),
    };

    // 各 provider 上次模型（对象；值须为非空字符串）。
    let model_by_provider = match obj.get("model_by_provider") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Object(o)) => {
            let mut map = BTreeMap::new();
            for (k, v) in o {
                let value = v
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "model_by_provider 的值须为非空字符串".to_string())?;
                map.insert(k.clone(), value.to_string());
            }
            Some(map)
        }
        Some(_) => return Err("model_by_provider 须为对象".to_string()),
    };

    Ok(ConfigPatch {
        provider,
        model,
        result_root,
        execution_enabled,
        max_uses,
        custom,
        model_by_provider,
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
        yaml_serde::from_str(storage::strip_bom(&text)).map_err(|e| format!("解析设置文件失败：{e}"))?
    } else {
        Value::Mapping(Mapping::new())
    };

    let current = parse_agent_config(&value);
    if patch.execution_enabled == Some(true) && !current.execution_enabled && !consent_cloud {
        return Err("启用本机执行需显式云端同意".to_string());
    }

    // custom 仅 provider=custom 时生效：此时须有有效自定义端点（protocol / http(s) URL）。
    // 其他 provider 下 custom 惰性保留、不校验（不报错）。
    let effective_provider = patch
        .provider
        .clone()
        .unwrap_or_else(|| current.provider.clone());
    if effective_provider == presets::CUSTOM_PROVIDER_ID {
        let custom = match &patch.custom {
            Some(CustomPatch::Set(c)) => Some(c.clone()),
            Some(CustomPatch::Clear) => None,
            None => current.custom.clone(),
        };
        let custom = custom.ok_or_else(|| "provider=custom 需先配置自定义端点".to_string())?;
        let protocol = parse_protocol_str(&custom.protocol)?;
        presets::resolve_custom(protocol, &custom.base_url)?;
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

    if let Some(provider) = patch.provider {
        agent.insert(storage::key("provider"), Value::String(provider));
    }
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
    if let Some(custom) = &patch.custom {
        match custom {
            CustomPatch::Set(c) => {
                let mut m = Mapping::new();
                m.insert(storage::key("protocol"), Value::String(c.protocol.clone()));
                m.insert(storage::key("base_url"), Value::String(c.base_url.clone()));
                agent.insert(storage::key("custom"), Value::Mapping(m));
            }
            CustomPatch::Clear => {
                agent.insert(storage::key("custom"), Value::Null);
            }
        }
    }
    if let Some(map) = &patch.model_by_provider {
        let mut m = Mapping::new();
        for (k, v) in map {
            m.insert(Value::String(k.clone()), Value::String(v.clone()));
        }
        agent.insert(storage::key("model_by_provider"), Value::Mapping(m));
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

    #[test]
    fn provider_defaults_roundtrip_and_rejects_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        // 缺失 → 默认 deepseek
        assert_eq!(load_agent_config_from(base).provider, "deepseek");

        // 空 / 纯空白 → 默认
        write_settings(base, "agent:\n  provider: '   '\n");
        assert_eq!(load_agent_config_from(base).provider, "deepseek");

        // 已知 → 原样且可解析预设
        write_settings(base, "agent:\n  provider: anthropic\n");
        let cfg = load_agent_config_from(base);
        assert_eq!(cfg.provider, "anthropic");
        assert_eq!(cfg.provider_preset().unwrap().id, "anthropic");

        // 往返保存
        save_agent_config_in(base, &json!({ "provider": "anthropic" }), false).unwrap();
        assert_eq!(load_agent_config_from(base).provider, "anthropic");

        // 未知 → 保存明确报错且不写
        let before = load_agent_config_from(base).provider;
        assert!(save_agent_config_in(base, &json!({ "provider": "bogus" }), false).is_err());
        assert_eq!(load_agent_config_from(base).provider, before);

        // 未知（手改文件）→ provider_preset 明确报错（fail closed）
        write_settings(base, "agent:\n  provider: bogus\n");
        assert!(load_agent_config_from(base).provider_preset().is_err());
    }

    #[test]
    fn custom_and_model_map_roundtrip_and_validation() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        // 缺失 → None / 空 map
        let cfg = load_agent_config_from(base);
        assert!(cfg.custom.is_none());
        assert!(cfg.model_by_provider.is_empty());

        // 往返保存（provider=custom 需有效 custom）
        save_agent_config_in(
            base,
            &json!({
                "provider": "custom",
                "custom": { "protocol": "openai", "base_url": "https://h.example/v1" },
                "model_by_provider": { "openai": "gpt-5.6-sol", "custom": "local-model" },
            }),
            false,
        )
        .unwrap();
        let cfg = load_agent_config_from(base);
        assert_eq!(cfg.provider, "custom");
        let c = cfg.custom.as_ref().unwrap();
        assert_eq!(c.protocol, "openai");
        assert_eq!(c.base_url, "https://h.example/v1");
        assert_eq!(
            cfg.model_by_provider.get("openai").map(String::as_str),
            Some("gpt-5.6-sol")
        );
        assert_eq!(
            cfg.model_by_provider.get("custom").map(String::as_str),
            Some("local-model")
        );
        assert!(cfg.resolved().is_ok());
        assert!(cfg.validate_provider().is_ok());

        // 非 custom provider 下 custom 惰性保留、不报错
        save_agent_config_in(base, &json!({ "provider": "openai" }), false).unwrap();
        let cfg = load_agent_config_from(base);
        assert_eq!(cfg.provider, "openai");
        assert!(cfg.custom.is_some(), "custom 惰性保留");
        assert!(cfg.validate_provider().is_ok());

        // provider=custom 但无 custom → 报错
        write_settings(base, "agent:\n  provider: custom\n");
        assert!(save_agent_config_in(base, &json!({ "provider": "custom" }), false).is_err());

        // custom 非法 protocol / base_url → 报错（provider=custom）
        assert!(save_agent_config_in(
            base,
            &json!({ "provider": "custom", "custom": { "protocol": "bogus", "base_url": "https://h" } }),
            false
        )
        .is_err());
        assert!(save_agent_config_in(
            base,
            &json!({ "provider": "custom", "custom": { "protocol": "openai", "base_url": "ftp://h" } }),
            false
        )
        .is_err());
        assert!(save_agent_config_in(
            base,
            &json!({ "provider": "custom", "custom": { "protocol": "openai", "base_url": "" } }),
            false
        )
        .is_err());

        // model_by_provider 反例
        assert!(save_agent_config_in(base, &json!({ "model_by_provider": { "openai": "" } }), false).is_err());
        assert!(save_agent_config_in(base, &json!({ "model_by_provider": { "openai": 3 } }), false).is_err());
        assert!(save_agent_config_in(base, &json!({ "model_by_provider": [] }), false).is_err());

        // custom 结构反例
        assert!(save_agent_config_in(base, &json!({ "custom": { "protocol": "openai" } }), false).is_err());
        assert!(save_agent_config_in(base, &json!({ "custom": "x" }), false).is_err());
    }

    #[test]
    fn custom_clear_requires_non_custom_provider() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        save_agent_config_in(
            base,
            &json!({ "provider": "custom", "custom": { "protocol": "anthropic", "base_url": "https://h" } }),
            false,
        )
        .unwrap();
        assert!(load_agent_config_from(base).custom.is_some());

        // provider 仍为 custom → 清空 custom 生效后缺失 → 报错
        assert!(save_agent_config_in(base, &json!({ "custom": null }), false).is_err());

        // 切回预设后再清空 → 允许（惰性保留语义）
        save_agent_config_in(base, &json!({ "provider": "deepseek" }), false).unwrap();
        save_agent_config_in(base, &json!({ "custom": null }), false).unwrap();
        assert!(load_agent_config_from(base).custom.is_none());
    }

    #[test]
    fn unknown_provider_still_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_settings(base, "agent:\n  provider: bogus\n");
        let cfg = load_agent_config_from(base);
        assert!(cfg.validate_provider().is_err());
        assert!(cfg.resolved().is_err());
    }

    #[test]
    fn settings_with_bom_parses_and_saves() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        // 带前导 BOM 的 settings：应正常解析 agent 段
        let text = "\u{FEFF}agent:\n  provider: anthropic\n  model: claude-sonnet-5\n";
        fs::write(base.join(storage::SETTINGS_FILE), text).unwrap();
        let cfg = load_agent_config_from(base);
        assert_eq!(cfg.provider, "anthropic");
        assert_eq!(cfg.model, "claude-sonnet-5");

        // 保存不再出现误导性「多文档」错误，且可往返
        save_agent_config_in(base, &json!({ "model": "m2" }), false).unwrap();
        let cfg = load_agent_config_from(base);
        assert_eq!(cfg.model, "m2");
        assert_eq!(cfg.provider, "anthropic");
    }
}
