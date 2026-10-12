//! Phase 1 冻结的 Agent 命令（薄编排；逻辑在 `config` / `intent` / `sandbox` / `secret`）。
//!
//! **密钥绝不回传**；`agent_save_config` / `agent_set_key` **绝不**自动联网；
//! probe 仅在 `confirmPaid === true` 时才会走网络。

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use zeroize::Zeroizing;

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
pub(crate) async fn probe_with_provider(
    confirm_paid: bool,
    injected: Option<Box<dyn provider::Provider>>,
) -> ProbeResult {
    if !confirm_paid {
        return ProbeResult {
            ok: false,
            error_class: Some(provider::CLASS_ABORTED.to_string()),
            message: "未确认费用，已取消（未发起网络请求）。".to_string(),
        };
    }
    let provider: Box<dyn provider::Provider> = match injected {
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
            match provider::DeepSeekProvider::new(store, cfg.model) {
                Ok(p) => Box::new(p),
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
        let result = probe_with_provider(false, Some(Box::new(spy))).await;
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
