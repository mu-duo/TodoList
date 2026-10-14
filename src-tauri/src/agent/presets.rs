//! Provider 预设注册表 + 统一解析（多 Provider，见 ADR-0009）。
//!
//! 预设是**唯一事实源**：前端经命令 `agent_list_providers` 获取，避免前后端双源。
//! `base_url` / 端点路径 / 认证 / 默认模型 / 搜索支持均由本表决定；[`resolve_provider`]
//! 将预设（+ 模型）解析为统一的 [`ResolvedProvider`] 供 provider 工厂构造。
//!
//! 混合协议（opencode-go）按**模型**映射协议；OpenAI 兼容协议在本表声明其 `chat_path`。

use serde::Serialize;

/// 协议族。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Anthropic Messages 兼容（`POST {base}{messages_path}`）。
    AnthropicCompat,
    /// OpenAI Chat Completions 兼容（`POST {base}{chat_path}`）。
    OpenAiCompat,
}

impl Protocol {
    /// 前端视图用字符串（`"anthropic"` / `"openai"`）。
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::AnthropicCompat => "anthropic",
            Protocol::OpenAiCompat => "openai",
        }
    }
}

/// 认证方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStyle {
    /// 以固定头名（[`ProviderPreset::auth_header`]）携带原始密钥（如 `x-api-key`）。
    ApiKeyHeader,
    /// `Authorization: Bearer <key>`。
    Bearer,
}

/// 一个内置 Provider 预设。
#[derive(Debug, Clone, Copy)]
pub struct ProviderPreset {
    /// 稳定 id（写入 `settings.yaml` 的 `agent.provider`）。
    pub id: &'static str,
    /// 展示名。
    pub label: &'static str,
    /// 标称协议（混合协议预设取默认侧；逐模型协议由 [`resolve_provider`] 决定）。
    pub protocol: Protocol,
    /// API base（不含端点路径）。
    pub base_url: &'static str,
    /// Anthropic 协议端点相对路径（拼接到 `base_url` 之后，以 `/` 开头）。
    pub messages_path: &'static str,
    /// OpenAI 协议端点相对路径（拼接到 `base_url` 之后，以 `/` 开头）。
    pub chat_path: &'static str,
    /// 预设默认模型；空串 = 无默认（需用户手输）。
    pub default_model: &'static str,
    /// 认证头名（值仍从本机凭据槽读取；明文永不回 JS）。
    pub auth_header: &'static str,
    /// 认证风格。
    pub auth_style: AuthStyle,
    /// 该预设是否声明支持服务端搜索（`web_search_20250305`）。
    pub search_support: bool,
    /// 动态模型列表端点（part 2 的 `agent_list_models` 消费）。
    #[allow(dead_code)]
    pub models_url: Option<&'static str>,
    /// 模型列表请求的认证风格（缺省 = [`ProviderPreset::auth_style`]）。DeepSeek 的 models
    /// 端点属 OpenAI 侧 → Bearer（与 chat 的 `x-api-key` 不同）。
    #[allow(dead_code)]
    pub models_auth: Option<AuthStyle>,
    /// 额外固定请求头（逐 run 动态头由 [`resolve_provider`] 追加）。
    pub extra_headers: &'static [(&'static str, &'static str)],
    /// 搜索请求需注入的 system 内容块（如 Qwen 的 billing header 约定）。
    pub search_system_hint: Option<&'static str>,
}

/// 默认预设 id（`agent.provider` 缺失 / 空时）。
pub const DEFAULT_PROVIDER_ID: &str = "deepseek";

/// 合成「自定义端点」provider id（不属 [`PRESETS`]，由 `settings.agent.custom` 提供端点）。
pub const CUSTOM_PROVIDER_ID: &str = "custom";

/// 内置预设表（顺序稳定）。
pub const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        id: "openai",
        label: "OpenAI",
        protocol: Protocol::OpenAiCompat,
        base_url: "https://api.openai.com/v1",
        messages_path: "",
        chat_path: "/chat/completions",
        default_model: "gpt-5.6-sol",
        auth_header: "authorization",
        auth_style: AuthStyle::Bearer,
        search_support: false,
        models_url: Some("https://api.openai.com/v1/models"),
        models_auth: None,
        extra_headers: &[],
        search_system_hint: None,
    },
    ProviderPreset {
        id: "deepseek",
        label: "DeepSeek",
        protocol: Protocol::AnthropicCompat,
        base_url: "https://api.deepseek.com/anthropic",
        // DeepSeek Anthropic 兼容端点：SDK 语义下再拼 `/v1/messages`（与现状一致）。
        messages_path: "/v1/messages",
        chat_path: "",
        default_model: "deepseek-flash",
        auth_header: "x-api-key",
        auth_style: AuthStyle::ApiKeyHeader,
        search_support: true,
        models_url: Some("https://api.deepseek.com/models"),
        // DeepSeek 的 models 端点属 OpenAI 侧 → Bearer（chat 用 x-api-key）。
        models_auth: Some(AuthStyle::Bearer),
        extra_headers: &[],
        search_system_hint: None,
    },
    ProviderPreset {
        id: "anthropic",
        label: "Anthropic",
        protocol: Protocol::AnthropicCompat,
        base_url: "https://api.anthropic.com/v1",
        messages_path: "/messages",
        chat_path: "",
        default_model: "claude-sonnet-5",
        auth_header: "x-api-key",
        auth_style: AuthStyle::ApiKeyHeader,
        search_support: true,
        models_url: Some("https://api.anthropic.com/v1/models"),
        models_auth: None,
        extra_headers: &[],
        search_system_hint: None,
    },
    ProviderPreset {
        id: "opencode-go",
        label: "OpenCode Go",
        // 混合协议：标称 OpenAI；messages 模型由 resolve_provider 覆写为 Anthropic。
        protocol: Protocol::OpenAiCompat,
        base_url: "https://opencode.ai/inference/go/v1",
        messages_path: "/messages",
        chat_path: "/chat/completions",
        default_model: "deepseek-v4.1-flash",
        auth_header: "authorization",
        auth_style: AuthStyle::Bearer,
        search_support: false,
        models_url: Some("https://opencode.ai/inference/go/v1/models"),
        models_auth: None,
        extra_headers: &[("User-Agent", "todolist-agent/1.0")],
        search_system_hint: None,
    },
    ProviderPreset {
        id: "openrouter",
        label: "OpenRouter",
        protocol: Protocol::OpenAiCompat,
        base_url: "https://openrouter.ai/api/v1",
        messages_path: "",
        chat_path: "/chat/completions",
        default_model: "anthropic/claude-sonnet-5",
        auth_header: "authorization",
        auth_style: AuthStyle::Bearer,
        search_support: false,
        models_url: Some("https://openrouter.ai/api/v1/models"),
        models_auth: None,
        extra_headers: &[],
        search_system_hint: None,
    },
    ProviderPreset {
        id: "gemini",
        label: "Gemini",
        protocol: Protocol::OpenAiCompat,
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        messages_path: "",
        chat_path: "/chat/completions",
        default_model: "gemini-3.5-flash",
        auth_header: "authorization",
        auth_style: AuthStyle::Bearer,
        search_support: false,
        models_url: Some("https://generativelanguage.googleapis.com/v1beta/openai/models"),
        models_auth: None,
        extra_headers: &[],
        search_system_hint: None,
    },
    ProviderPreset {
        id: "qwen",
        label: "Qwen",
        protocol: Protocol::AnthropicCompat,
        base_url: "https://dashscope.aliyuncs.com/apps/anthropic",
        messages_path: "/v1/messages",
        chat_path: "",
        default_model: "qwen3.8-max",
        auth_header: "authorization",
        auth_style: AuthStyle::Bearer,
        search_support: true,
        models_url: None,
        models_auth: None,
        extra_headers: &[],
        // 官方约定：billing header 须作为 system 内容块传入，否则 web_search 不生效。
        search_system_hint: Some("x-anthropic-billing-header: cc_entrypoint=cli;"),
    },
    ProviderPreset {
        id: "zhipu",
        label: "Zhipu",
        protocol: Protocol::AnthropicCompat,
        base_url: "https://open.bigmodel.cn/api/anthropic",
        messages_path: "/v1/messages",
        chat_path: "",
        default_model: "glm-5.3",
        auth_header: "authorization",
        auth_style: AuthStyle::Bearer,
        // 待验证 → 先降级为不支持。
        search_support: false,
        models_url: None,
        models_auth: None,
        extra_headers: &[],
        search_system_hint: None,
    },
    ProviderPreset {
        id: "kimi",
        label: "Kimi",
        protocol: Protocol::OpenAiCompat,
        base_url: "https://api.moonshot.ai/v1",
        messages_path: "",
        chat_path: "/chat/completions",
        default_model: "kimi-k3",
        auth_header: "authorization",
        auth_style: AuthStyle::Bearer,
        search_support: false,
        models_url: Some("https://api.moonshot.ai/v1/models"),
        models_auth: None,
        extra_headers: &[],
        search_system_hint: None,
    },
];

/// 按 id 查预设；未知返回 `None`。
pub fn find(id: &str) -> Option<&'static ProviderPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// 归一化并解析 provider id：缺失 / 空 → 默认；未知 → 明确报错（fail closed）。
pub fn resolve(id: Option<&str>) -> Result<&'static ProviderPreset, String> {
    let id = id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_PROVIDER_ID);
    find(id).ok_or_else(|| format!("未知的 provider：{id}"))
}

// ---------------------------------------------------------------------------
// opencode-go 混合协议：按模型精确映射（官方文档 2026-10-14）
// ---------------------------------------------------------------------------

/// OpenCode Go：`chat/completions` 组（OpenAI 兼容）。
///
/// 来源：OpenCode Go 官方端点文档（2026-10-14）。**可能漂移，需随官方文档更新。**
const OPENCODE_GO_CHAT_MODELS: &[&str] = &[
    "glm-5.3-flash",
    "glm-5.3",
    "glm-5.2",
    "kimi-k3",
    "kimi-k2.7-code",
    "kimi-k2.6",
    "longcat-2.0",
    "deepseek-v4.1-flash",
    "deepseek-v4-pro",
    "deepseek-v4-flash",
    "deepseek-v4-flash-vision-exp",
    "mimo-v2.6-flash",
    "mimo-v2.6-pro",
    "mimo-v2.5",
    "mimo-v2.5-pro",
    "hy4-preview",
    "hy3",
    "space-bunny",
    "longcat-2.5-preview-free",
    "step-5-preview-free",
    "cat-jam-free",
];

/// OpenCode Go：Anthropic `messages` 组。来源同上，可能漂移。
const OPENCODE_GO_MESSAGES_MODELS: &[&str] = &[
    "claude-haiku-5-5",
    "minimax-m3",
    "minimax-m2.7",
    "qwen3.8-max",
    "qwen3.8-flash",
    "qwen3.7-plus",
];

/// OpenCode Go：OpenAI `responses` 组（v1 不支持 → 构造报错 / 列表标注不可用）。来源同上。
const OPENCODE_GO_RESPONSES_MODELS: &[&str] = &[
    "grok-4.7",
    "grok-4.6",
    "gpt-6-luna",
    "gpt-5.6-luna",
    "muse-spark-1.3-contributor",
    "muse-spark-1.2-contributor",
];

/// 解析 opencode-go 模型协议（精确匹配）：messages → Anthropic；responses → 报错（fail closed）；
/// chat 组与一切未知（含官方未列 6 个）→ OpenAI 兼容默认。
fn opencode_go_protocol(model: &str) -> Result<Protocol, String> {
    let m = model.trim();
    if OPENCODE_GO_RESPONSES_MODELS.contains(&m) {
        return Err(format!("opencode-go 的 responses 模型暂不支持：{model}"));
    }
    if OPENCODE_GO_MESSAGES_MODELS.contains(&m) {
        return Ok(Protocol::AnthropicCompat);
    }
    if OPENCODE_GO_CHAT_MODELS.contains(&m) {
        return Ok(Protocol::OpenAiCompat);
    }
    // 官方未列 / 未知 → OpenAI 默认。
    Ok(Protocol::OpenAiCompat)
}

/// 模型列表逐项标注：`(protocol, supported)`。responses → 不可用；其余（含未列 / 未知）→ 可用。
#[allow(dead_code)] // 仅非 feature 实现（真实拉取路径）与测试使用。
pub fn opencode_go_model_status(model: &str) -> (&'static str, bool) {
    let m = model.trim();
    if OPENCODE_GO_RESPONSES_MODELS.contains(&m) {
        ("openai", false)
    } else if OPENCODE_GO_MESSAGES_MODELS.contains(&m) {
        ("anthropic", true)
    } else {
        ("openai", true)
    }
}

// ---------------------------------------------------------------------------
// 统一解析：预设（+ 模型）→ ResolvedProvider
// ---------------------------------------------------------------------------

/// 已解析的 provider 规格：预设与自定义端点均由调用方解析成该规格。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProvider {
    pub provider_id: String,
    pub protocol: Protocol,
    pub base_url: String,
    /// chat 端点相对路径（Anthropic: messages_path；OpenAI: chat_path）。
    pub path: String,
    pub auth_style: AuthStyle,
    pub auth_header: String,
    pub extra_headers: Vec<(String, String)>,
    pub search_support: bool,
    pub search_system_hint: Option<String>,
    /// 是否必须有密钥：预设 = true；自定义端点 = false（key 可空）。
    pub key_required: bool,
}

/// 预设的额外请求头（含 opencode-go 的**每 run 会话标识**）。`resolve_provider` 与模型列表
/// 请求共用此函数，避免会话头双实现。
pub fn preset_extra_headers(preset: &ProviderPreset) -> Vec<(String, String)> {
    let mut headers: Vec<(String, String)> = preset
        .extra_headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if preset.id == "opencode-go" {
        // 每 run 会话标识（随机；见 ADR-0009 预设内置头）。ULID 提供足够的随机唯一性。
        headers.push((
            "x-opencode-session".to_string(),
            ulid::Ulid::generate().to_string(),
        ));
    }
    headers
}

/// 将预设 + 模型解析为统一规格；opencode-go 按模型映射协议，responses 组报错（fail closed）。
pub fn resolve_provider(
    preset: &ProviderPreset,
    model: &str,
) -> Result<ResolvedProvider, String> {
    let protocol = if preset.id == "opencode-go" {
        opencode_go_protocol(model)?
    } else {
        preset.protocol
    };
    let path = match protocol {
        Protocol::AnthropicCompat => preset.messages_path,
        Protocol::OpenAiCompat => preset.chat_path,
    };
    if path.is_empty() {
        return Err(format!(
            "预设 {} 缺少 {} 端点路径",
            preset.id,
            protocol.as_str()
        ));
    }
    let extra_headers = preset_extra_headers(preset);
    Ok(ResolvedProvider {
        provider_id: preset.id.to_string(),
        protocol,
        base_url: preset.base_url.to_string(),
        path: path.to_string(),
        auth_style: preset.auth_style,
        auth_header: preset.auth_header.to_string(),
        extra_headers,
        search_support: preset.search_support,
        search_system_hint: preset.search_system_hint.map(str::to_string),
        key_required: true,
    })
}

/// 自定义端点 → [`ResolvedProvider`]。
///
/// - `openai`：`base + /chat/completions`；`anthropic`：base 以 `/v1` 结尾 → `base + /messages`，
///   否则 `base + /v1/messages`（先去掉尾部 `/`）。
/// - 认证：Anthropic → `x-api-key`（与预设一致）；OpenAI → Bearer。
/// - `search_support=false`；`key_required=false`（key 可空）。
pub fn resolve_custom(protocol: Protocol, base_url: &str) -> Result<ResolvedProvider, String> {
    let base = base_url.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err("自定义端点地址不能为空".to_string());
    }
    // scheme 校验大小写不敏感（与前端一致）。
    let lower = base.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("自定义端点地址须为 http(s) URL".to_string());
    }
    let (path, auth_style, auth_header) = match protocol {
        Protocol::OpenAiCompat => (
            "/chat/completions".to_string(),
            AuthStyle::Bearer,
            "authorization",
        ),
        Protocol::AnthropicCompat => {
            let path = if lower.ends_with("/v1") {
                "/messages"
            } else {
                "/v1/messages"
            };
            (path.to_string(), AuthStyle::ApiKeyHeader, "x-api-key")
        }
    };
    Ok(ResolvedProvider {
        provider_id: "custom".to_string(),
        protocol,
        base_url: base,
        path,
        auth_style,
        auth_header: auth_header.to_string(),
        extra_headers: Vec::new(),
        search_support: false,
        search_system_hint: None,
        key_required: false,
    })
}

/// 自定义端点的模型列表 URL（与 [`resolve_custom`] 的 `/v1` 规则一致）。
#[allow(dead_code)] // 仅非 feature 实现（真实拉取路径）与测试使用。
pub fn custom_models_url(protocol: Protocol, base_url: &str) -> Option<String> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    Some(match protocol {
        Protocol::OpenAiCompat => format!("{base}/models"),
        Protocol::AnthropicCompat => {
            if base.ends_with("/v1") {
                format!("{base}/models")
            } else {
                format!("{base}/v1/models")
            }
        }
    })
}

// ---------------------------------------------------------------------------
// 前端视图
// ---------------------------------------------------------------------------

/// 预设的前端视图（**不含**密钥）。
#[derive(Debug, Clone, Serialize)]
pub struct PresetView {
    pub id: &'static str,
    pub label: &'static str,
    /// `"anthropic"` / `"openai"`；合成「自定义」条目为 `""`。
    pub protocol: &'static str,
    pub default_model: &'static str,
    pub search_support: bool,
    pub is_custom: bool,
}

/// 全部预设的前端视图（顺序稳定）+ 末尾合成「自定义」条目。
pub fn views() -> Vec<PresetView> {
    let mut views: Vec<PresetView> = PRESETS
        .iter()
        .map(|p| PresetView {
            id: p.id,
            label: p.label,
            protocol: p.protocol.as_str(),
            default_model: p.default_model,
            search_support: p.search_support,
            is_custom: false,
        })
        .collect();
    // 合成条目：自定义端点（part 2 实现 config.custom；此处仅提供下拉项）。
    views.push(PresetView {
        id: "custom",
        label: "自定义",
        protocol: "",
        default_model: "",
        search_support: false,
        is_custom: true,
    });
    views
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_known_and_unknown() {
        assert_eq!(find("deepseek").unwrap().label, "DeepSeek");
        assert_eq!(find("anthropic").unwrap().label, "Anthropic");
        assert_eq!(find("openai").unwrap().label, "OpenAI");
        assert!(find("nope").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn resolve_defaults_and_rejects_unknown() {
        assert_eq!(resolve(None).unwrap().id, DEFAULT_PROVIDER_ID);
        assert_eq!(resolve(Some("")).unwrap().id, DEFAULT_PROVIDER_ID);
        assert_eq!(resolve(Some("   ")).unwrap().id, DEFAULT_PROVIDER_ID);
        assert_eq!(resolve(Some("anthropic")).unwrap().id, "anthropic");
        let err = resolve(Some("bogus")).unwrap_err();
        assert!(err.contains("bogus"), "错误应含未知 id：{err}");
    }

    #[test]
    fn all_nine_presets_present_in_order() {
        let ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            vec![
                "openai",
                "deepseek",
                "anthropic",
                "opencode-go",
                "openrouter",
                "gemini",
                "qwen",
                "zhipu",
                "kimi"
            ]
        );
        assert_eq!(PRESETS.len(), 9);
    }

    #[test]
    fn preset_protocol_path_auth_and_default_model() {
        let cases: &[(&str, Protocol, &str, &str, AuthStyle, &str, bool)] = &[
            ("openai", Protocol::OpenAiCompat, "https://api.openai.com/v1", "/chat/completions", AuthStyle::Bearer, "gpt-5.6-sol", false),
            ("deepseek", Protocol::AnthropicCompat, "https://api.deepseek.com/anthropic", "/v1/messages", AuthStyle::ApiKeyHeader, "deepseek-flash", true),
            ("anthropic", Protocol::AnthropicCompat, "https://api.anthropic.com/v1", "/messages", AuthStyle::ApiKeyHeader, "claude-sonnet-5", true),
            ("opencode-go", Protocol::OpenAiCompat, "https://opencode.ai/inference/go/v1", "/chat/completions", AuthStyle::Bearer, "deepseek-v4.1-flash", false),
            ("openrouter", Protocol::OpenAiCompat, "https://openrouter.ai/api/v1", "/chat/completions", AuthStyle::Bearer, "anthropic/claude-sonnet-5", false),
            ("gemini", Protocol::OpenAiCompat, "https://generativelanguage.googleapis.com/v1beta/openai", "/chat/completions", AuthStyle::Bearer, "gemini-3.5-flash", false),
            ("qwen", Protocol::AnthropicCompat, "https://dashscope.aliyuncs.com/apps/anthropic", "/v1/messages", AuthStyle::Bearer, "qwen3.8-max", true),
            ("zhipu", Protocol::AnthropicCompat, "https://open.bigmodel.cn/api/anthropic", "/v1/messages", AuthStyle::Bearer, "glm-5.3", false),
            ("kimi", Protocol::OpenAiCompat, "https://api.moonshot.ai/v1", "/chat/completions", AuthStyle::Bearer, "kimi-k3", false),
        ];
        for (id, protocol, base, path, auth, model, search) in cases {
            let p = find(id).unwrap();
            assert_eq!(p.protocol, *protocol, "{id} protocol");
            assert_eq!(p.base_url, *base, "{id} base_url");
            assert_eq!(p.default_model, *model, "{id} default_model");
            assert_eq!(p.auth_style, *auth, "{id} auth_style");
            assert_eq!(p.search_support, *search, "{id} search_support");
            let spec = resolve_provider(p, p.default_model).unwrap();
            assert_eq!(&spec.path, path, "{id} resolved path");
            assert_eq!(spec.protocol, *protocol, "{id} resolved protocol");
        }
    }

    #[test]
    fn qwen_has_search_hint_and_no_models_url() {
        let qwen = find("qwen").unwrap();
        assert!(qwen.search_support);
        assert_eq!(
            qwen.search_system_hint,
            Some("x-anthropic-billing-header: cc_entrypoint=cli;")
        );
        assert!(qwen.models_url.is_none());
    }

    #[test]
    fn opencode_go_official_protocol_mapping() {
        let p = find("opencode-go").unwrap();
        // chat 组抽样 → OpenAI（chat/completions）
        for m in [
            "deepseek-v4.1-flash",
            "deepseek-v4-pro",
            "glm-5.3",
            "kimi-k3",
            "space-bunny",
        ] {
            let spec = resolve_provider(p, m).unwrap();
            assert_eq!(spec.protocol, Protocol::OpenAiCompat, "{m}");
            assert_eq!(spec.path, "/chat/completions", "{m}");
        }
        // messages 组抽样 → Anthropic（messages）
        for m in ["claude-haiku-5-5", "qwen3.8-max", "minimax-m3", "qwen3.7-plus"] {
            let spec = resolve_provider(p, m).unwrap();
            assert_eq!(spec.protocol, Protocol::AnthropicCompat, "{m}");
            assert_eq!(spec.path, "/messages", "{m}");
        }
        // responses 组抽样 → 报错（fail closed）
        for m in ["grok-4.7", "gpt-6-luna", "muse-spark-1.2-contributor"] {
            assert!(resolve_provider(p, m).is_err(), "{m} 应报错");
        }
        // 官方未列 6 个 + 未知 → OpenAI 默认
        for m in [
            "deepseek-flash",
            "glm-5.1",
            "omen-alpha",
            "minimax-m2.5",
            "qwen3.6-plus",
            "qwen3.7-max",
            "mystery-model",
        ] {
            assert_eq!(
                resolve_provider(p, m).unwrap().protocol,
                Protocol::OpenAiCompat,
                "{m} 应默认 OpenAI"
            );
        }
        // 额外头：User-Agent + 每 run 会话 id
        let spec = resolve_provider(p, "deepseek-v4.1-flash").unwrap();
        assert!(spec
            .extra_headers
            .iter()
            .any(|(k, v)| k == "User-Agent" && v == "todolist-agent/1.0"));
        let session = spec
            .extra_headers
            .iter()
            .find(|(k, _)| k == "x-opencode-session")
            .map(|(_, v)| v.clone())
            .expect("应有会话头");
        assert!(!session.is_empty());
        // 两次解析会话不同（每 run 随机）
        let spec2 = resolve_provider(p, "deepseek-v4.1-flash").unwrap();
        assert_ne!(
            spec.extra_headers
                .iter()
                .find(|(k, _)| k == "x-opencode-session")
                .unwrap()
                .1,
            spec2
                .extra_headers
                .iter()
                .find(|(k, _)| k == "x-opencode-session")
                .unwrap()
                .1
        );
    }

    #[test]
    fn opencode_go_model_status_annotation() {
        assert_eq!(
            opencode_go_model_status("claude-haiku-5-5"),
            ("anthropic", true)
        );
        assert_eq!(opencode_go_model_status("deepseek-v4-pro"), ("openai", true));
        assert_eq!(opencode_go_model_status("grok-4.7"), ("openai", false));
        assert_eq!(opencode_go_model_status("deepseek-flash"), ("openai", true));
        assert_eq!(opencode_go_model_status("mystery"), ("openai", true));
    }

    #[test]
    fn resolve_custom_paths_auth_and_validation() {
        // openai → /chat/completions + Bearer
        let s = resolve_custom(Protocol::OpenAiCompat, "https://host.example/v1/").unwrap();
        assert_eq!(s.provider_id, "custom");
        assert_eq!(s.base_url, "https://host.example/v1");
        assert_eq!(s.path, "/chat/completions");
        assert_eq!(s.auth_style, AuthStyle::Bearer);
        assert_eq!(s.auth_header, "authorization");
        assert!(!s.search_support);
        assert!(!s.key_required, "自定义端点 key 可空");

        // anthropic：base 不以 /v1 结尾 → /v1/messages；认证 x-api-key（与预设一致）
        let s = resolve_custom(Protocol::AnthropicCompat, "https://host.example").unwrap();
        assert_eq!(s.path, "/v1/messages");
        assert_eq!(s.auth_style, AuthStyle::ApiKeyHeader);
        assert_eq!(s.auth_header, "x-api-key");
        // anthropic：base 以 /v1 结尾 → /messages
        let s = resolve_custom(Protocol::AnthropicCompat, "https://host.example/v1/").unwrap();
        assert_eq!(s.base_url, "https://host.example/v1");
        assert_eq!(s.path, "/messages");

        // scheme 大小写不敏感（与前端一致）
        let s = resolve_custom(Protocol::OpenAiCompat, "HTTPS://Host.Example/V1").unwrap();
        assert_eq!(s.base_url, "HTTPS://Host.Example/V1");
        assert_eq!(s.path, "/chat/completions");
        let s = resolve_custom(Protocol::AnthropicCompat, "HTTP://Host.Example/V1").unwrap();
        assert_eq!(s.path, "/messages");

        // 校验：空 / 非 http(s) → 报错
        assert!(resolve_custom(Protocol::OpenAiCompat, "").is_err());
        assert!(resolve_custom(Protocol::OpenAiCompat, "   ").is_err());
        assert!(resolve_custom(Protocol::OpenAiCompat, "ftp://host").is_err());
        assert!(resolve_custom(Protocol::OpenAiCompat, "host.example/v1").is_err());
    }

    #[test]
    fn deepseek_models_auth_is_bearer() {
        let ds = find("deepseek").unwrap();
        assert_eq!(ds.models_auth, Some(AuthStyle::Bearer), "models 属 OpenAI 侧");
        assert_eq!(ds.auth_style, AuthStyle::ApiKeyHeader, "chat 仍 x-api-key");
        // 其余预设缺省（= auth_style）
        assert_eq!(find("openai").unwrap().models_auth, None);
        assert_eq!(find("anthropic").unwrap().models_auth, None);
    }

    #[test]
    fn preset_extra_headers_include_opencode_session() {
        let p = find("opencode-go").unwrap();
        let h = preset_extra_headers(p);
        assert!(h
            .iter()
            .any(|(k, v)| k == "User-Agent" && v == "todolist-agent/1.0"));
        assert!(h.iter().any(|(k, _)| k == "x-opencode-session"));
        // 两次生成会话不同（每 run 随机）
        let h2 = preset_extra_headers(p);
        assert_ne!(
            h.iter()
                .find(|(k, _)| k == "x-opencode-session")
                .unwrap()
                .1,
            h2.iter()
                .find(|(k, _)| k == "x-opencode-session")
                .unwrap()
                .1
        );
        // 非 opencode-go：无会话头
        let ds = preset_extra_headers(find("deepseek").unwrap());
        assert!(ds.iter().all(|(k, _)| k != "x-opencode-session"));
    }

    #[test]
    fn custom_models_url_follows_v1_rule() {
        assert_eq!(
            custom_models_url(Protocol::OpenAiCompat, "https://h/v1/").as_deref(),
            Some("https://h/v1/models")
        );
        assert_eq!(
            custom_models_url(Protocol::AnthropicCompat, "https://h").as_deref(),
            Some("https://h/v1/models")
        );
        assert_eq!(
            custom_models_url(Protocol::AnthropicCompat, "https://h/v1").as_deref(),
            Some("https://h/v1/models")
        );
        assert_eq!(custom_models_url(Protocol::OpenAiCompat, ""), None);
    }

    #[test]
    fn views_expose_protocol_and_custom_entry_without_secrets() {
        let views = views();
        assert_eq!(views.len(), PRESETS.len() + 1);
        assert_eq!(views[0].id, "openai");
        assert_eq!(views[0].protocol, "openai");
        assert_eq!(views[1].id, "deepseek");
        assert_eq!(views[1].protocol, "anthropic");
        assert_eq!(views[2].id, "anthropic");
        assert_eq!(views[2].protocol, "anthropic");
        // 合成自定义条目
        let last = views.last().unwrap();
        assert_eq!(last.id, "custom");
        assert!(last.is_custom);
        assert_eq!(last.protocol, "");
        // 内置条目非自定义
        assert!(views.iter().take(PRESETS.len()).all(|v| !v.is_custom));
        // 视图无密钥字段
        let json = serde_json::to_string(&views).unwrap();
        assert!(!json.contains("\"key\""));
    }
}
