//! Provider：主 chat 请求与**独立** native web_search 请求（禁混用、禁跟随重定向）。
//!
//! 请求构造与响应解析均为纯函数（离线可测）；HTTP 仅经 [`AnthropicCompatProvider`] /
//! [`OpenAiCompatProvider`] 走 Rust。错误分类用于 `error_class`，**绝不**包含密钥或原始响应体。
//!
//! 端点 / 认证 / 凭据槽由**预设 / 配置**经 [`ResolvedProvider`] 提供（多 Provider，见 ADR-0009）；
//! 消息与工具为**协议无关**表示，各适配器负责 wire 转换。安全不变量集中在 [`send_json`]：禁重定向、
//! 30 分钟兜底超时、绝对 deadline、4MiB 上限、取消。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use super::presets::{AuthStyle, ResolvedProvider};
use super::secret::SecretStore;

/// Anthropic 兼容协议版本。
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// 服务端搜索工具类型（固定版本名）。
#[allow(dead_code)] // feature 构建下 AnthropicCompatProvider 搜索路径不被工厂使用。
pub const SEARCH_TOOL_TYPE: &str = "web_search_20250305";
/// 服务端搜索工具名。
#[allow(dead_code)] // feature 构建下 AnthropicCompatProvider 搜索路径不被工厂使用。
pub const SEARCH_TOOL_NAME: &str = "web_search";
/// chat 默认 max_tokens。
#[allow(dead_code)] // P2 runner：主 chat 循环消费。
pub const CHAT_MAX_TOKENS: u32 = 4096;
/// 辅助 search 默认 max_tokens（小）。
#[allow(dead_code)] // feature 构建下 AnthropicCompatProvider 搜索路径不被工厂使用。
pub const SEARCH_MAX_TOKENS: u32 = 1024;
/// 响应体字节上限（防止内存放大 / 日志泄漏）。
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// 客户端整体超时（兜底；单请求另设 `deadline`）。
#[allow(dead_code)] // feature 构建下 AnthropicCompatProvider 不被工厂使用。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub const CLASS_MISSING_CREDENTIAL: &str = "MissingCredential";
pub const CLASS_HTTP_AUTH: &str = "HttpAuth";
pub const CLASS_HTTP_STATUS: &str = "HttpStatus";
pub const CLASS_TRANSPORT: &str = "Transport";
pub const CLASS_REDIRECT: &str = "Redirect";
pub const CLASS_PARSE: &str = "Parse";
pub const CLASS_NO_RESULT_BLOCK: &str = "NoResultBlock";
pub const CLASS_ABORTED: &str = "Aborted";
/// 2xx 响应体内含上游 `error` 对象（**区别于** `HttpStatus`，供 P2 判定致命而非可重试）。
pub const CLASS_PROVIDER_ERROR: &str = "ProviderError";
/// 配置错误（如未知 provider）；区别于传输 / 鉴权 / 状态。
pub const CLASS_CONFIG: &str = "Config";

/// 编译期事实：本构建的 [`make_provider`] 是否返回假实现。
///
/// 仅 `agent-test-provider` feature 开启时为 `true`；默认 / release 构建恒为 `false`（编译期确定，
/// **不是**运行时 env 开关）。供测试断言“默认构建绝不返回假实现”。
#[allow(dead_code)]
pub const TEST_PROVIDER_ENABLED: bool = cfg!(feature = "agent-test-provider");

/// 归一化脚本中的错误类名到内部 `&'static str` 常量（仅 fixture 脚本解析用）。
#[cfg(any(test, feature = "agent-test-provider"))]
#[allow(dead_code)]
pub fn class_name_to_static(name: &str) -> &'static str {
    match name {
        "MissingCredential" => CLASS_MISSING_CREDENTIAL,
        "HttpAuth" => CLASS_HTTP_AUTH,
        "HttpStatus" => CLASS_HTTP_STATUS,
        "Transport" => CLASS_TRANSPORT,
        "Redirect" => CLASS_REDIRECT,
        "Parse" => CLASS_PARSE,
        "NoResultBlock" => CLASS_NO_RESULT_BLOCK,
        "Aborted" => CLASS_ABORTED,
        "Config" => CLASS_CONFIG,
        _ => CLASS_PROVIDER_ERROR,
    }
}

/// Provider 错误（分类 + 安全消息，不含密钥 / 原始响应体）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    class: &'static str,
    message: String,
}

impl ProviderError {
    pub fn new(class: &'static str, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
        }
    }
    pub fn class(&self) -> &'static str {
        self.class
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// 取消令牌（轻量；Phase 2 runner 复用）。
#[derive(Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    #[allow(dead_code)] // P2 runner：用户取消 / 停止。
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

async fn wait_cancel(token: CancelToken) {
    while !token.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ---------------------------------------------------------------------------
// 协议无关的对话 / 工具表示（runner 主循环据此往返；各协议适配器负责转换）
// ---------------------------------------------------------------------------

/// 客户端工具声明（协议无关）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// 客户端工具调用（协议无关；`args` 为工具入参对象）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

/// 工具执行结果（协议无关；回填给模型）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub id: String,
    pub content: String,
    pub is_error: bool,
}

/// 协议无关的一条对话消息。
#[derive(Debug, Clone, PartialEq)]
pub enum ChatMessage {
    /// 用户文本。
    User(String),
    /// 助手回合（文本 + 工具调用）。
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    /// 工具执行结果回填。
    ToolResults(Vec<ToolResult>),
}

/// 一条归一化搜索来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchSource {
    pub url: String,
    pub title: Option<String>,
    pub page_age: Option<String>,
    pub snippet: Option<String>,
}

/// search 结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchOutcome {
    pub sources: Vec<SearchSource>,
    pub text: String,
    pub had_server_tool_use: bool,
}

/// chat 结果（宽容：citations / server_tool_use 缺失均可）。
#[allow(dead_code)] // P2 runner：主 chat 循环消费。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatOutcome {
    pub text: String,
    pub had_server_tool_use: bool,
    pub had_search_result: bool,
    pub citations: Vec<String>,
    /// 客户端工具调用（协议无关）；供 runner 执行 fs / native_search。
    pub tool_calls: Vec<ToolCall>,
}

// ---------------------------------------------------------------------------
// Anthropic Messages 兼容：wire 构造（纯函数，离线可测）
// ---------------------------------------------------------------------------

/// Anthropic：协议无关消息 → wire `messages`。
#[allow(dead_code)] // feature 构建下由假实现替代；测试仍直接消费。
pub fn anthropic_messages(messages: &[ChatMessage]) -> Value {
    Value::Array(
        messages
            .iter()
            .map(|m| match m {
                ChatMessage::User(text) => {
                    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
                }
                ChatMessage::Assistant { text, tool_calls } => {
                    let mut blocks: Vec<Value> = Vec::new();
                    if !text.is_empty() {
                        blocks.push(json!({ "type": "text", "text": text }));
                    }
                    for c in tool_calls {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": c.id,
                            "name": c.name,
                            "input": c.args,
                        }));
                    }
                    json!({ "role": "assistant", "content": Value::Array(blocks) })
                }
                ChatMessage::ToolResults(results) => json!({
                    "role": "user",
                    "content": results
                        .iter()
                        .map(|r| json!({
                            "type": "tool_result",
                            "tool_use_id": r.id,
                            "content": r.content,
                            "is_error": r.is_error,
                        }))
                        .collect::<Vec<_>>(),
                }),
            })
            .collect(),
    )
}

/// Anthropic：协议无关工具 → wire `tools`。
#[allow(dead_code)] // feature 构建下由假实现替代；测试仍直接消费。
pub fn anthropic_tools(tools: &[ToolSpec]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect(),
    )
}

/// 构造主 chat 请求（Anthropic wire；**不**含服务端 web_search）。`tools` 为空时不发送
/// `tools` 字段（与 OpenAI 对齐）。
#[allow(dead_code)] // P2 runner：主 chat 循环消费。
pub fn build_chat_request(model: &str, messages: Value, tools: Value, max_tokens: u32) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages,
    });
    if tools.as_array().is_some_and(|a| !a.is_empty()) {
        body["tools"] = tools;
    }
    body
}

/// 构造**独立** native search 请求（Anthropic wire）：唯一工具为服务端 `web_search_20250305`。
/// `system_hint` 非空时注入顶层 `system` 内容块（如 Qwen 的 billing header 约定）。
#[allow(dead_code)] // feature 构建下由假实现替代；测试仍直接消费。
pub fn build_search_request(
    model: &str,
    query: &str,
    max_uses: u32,
    max_tokens: u32,
    system_hint: Option<&str>,
) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": query }] }],
        "tools": [{ "type": SEARCH_TOOL_TYPE, "name": SEARCH_TOOL_NAME, "max_uses": max_uses }],
    });
    if let Some(hint) = system_hint.filter(|h| !h.is_empty()) {
        body["system"] = json!([{ "type": "text", "text": hint }]);
    }
    body
}

// ---------------------------------------------------------------------------
// OpenAI Chat Completions 兼容：wire 构造（纯函数，离线可测）
// ---------------------------------------------------------------------------

/// OpenAI：协议无关消息 → wire `messages`（工具结果拆为 `role:"tool"` 消息）。
#[allow(dead_code)] // feature 构建下由假实现替代；测试仍直接消费。
pub fn openai_messages(messages: &[ChatMessage]) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for m in messages {
        match m {
            ChatMessage::User(text) => out.push(json!({ "role": "user", "content": text })),
            ChatMessage::Assistant { text, tool_calls } => {
                if tool_calls.is_empty() {
                    out.push(json!({ "role": "assistant", "content": text }));
                } else {
                    out.push(json!({
                        "role": "assistant",
                        "content": if text.is_empty() { Value::Null } else { Value::String(text.clone()) },
                        "tool_calls": tool_calls
                            .iter()
                            .map(|c| json!({
                                "id": c.id,
                                "type": "function",
                                "function": { "name": c.name, "arguments": c.args.to_string() },
                            }))
                            .collect::<Vec<_>>(),
                    }));
                }
            }
            ChatMessage::ToolResults(results) => {
                for r in results {
                    out.push(json!({
                        "role": "tool",
                        "tool_call_id": r.id,
                        "content": r.content,
                    }));
                }
            }
        }
    }
    Value::Array(out)
}

/// OpenAI：协议无关工具 → wire `tools`（`type:"function"`）。
#[allow(dead_code)] // feature 构建下由假实现替代；测试仍直接消费。
pub fn openai_tools(tools: &[ToolSpec]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    },
                })
            })
            .collect(),
    )
}

/// 构造 OpenAI 兼容 chat 请求；`tools` 为空时不发送 `tools` 字段。
#[allow(dead_code)] // feature 构建下由假实现替代；测试仍直接消费。
pub fn build_openai_chat_request(
    model: &str,
    messages: Value,
    tools: Value,
    max_tokens: u32,
) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages,
    });
    if tools.as_array().is_some_and(|a| !a.is_empty()) {
        body["tools"] = tools;
    }
    body
}

fn classify_status(status: u16) -> Result<(), ProviderError> {
    if (300..400).contains(&status) {
        return Err(ProviderError::new(
            CLASS_REDIRECT,
            "上游返回重定向，已拒绝跟随",
        ));
    }
    if status == 401 || status == 403 {
        return Err(ProviderError::new(
            CLASS_HTTP_AUTH,
            format!("上游鉴权失败（状态 {status}）"),
        ));
    }
    if !(200..300).contains(&status) {
        return Err(ProviderError::new(
            CLASS_HTTP_STATUS,
            format!("上游返回状态 {status}"),
        ));
    }
    Ok(())
}

/// 解析 search 响应：**无结果块或空来源 ⇒ 失败**，绝不判为联网成功。
#[allow(dead_code)] // feature 构建下由假实现替代；测试仍直接消费。
pub fn parse_search_response(status: u16, body: &str) -> Result<SearchOutcome, ProviderError> {
    classify_status(status)?;
    let value: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::new(CLASS_PARSE, "响应不是有效 JSON"))?;
    if value.get("error").is_some() {
        return Err(ProviderError::new(CLASS_PROVIDER_ERROR, "上游返回错误对象"));
    }
    let content = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::new(CLASS_PARSE, "响应缺少 content"))?;

    let mut citations: Vec<(String, String)> = Vec::new();
    let mut text = String::new();
    let mut had_server_tool_use = false;
    let mut sources: Vec<SearchSource> = Vec::new();

    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
                if let Some(cs) = block.get("citations").and_then(Value::as_array) {
                    for c in cs {
                        if let (Some(url), Some(cited)) = (
                            c.get("url").and_then(Value::as_str),
                            c.get("cited_text").and_then(Value::as_str),
                        ) {
                            citations.push((url.to_string(), cited.to_string()));
                        }
                    }
                }
            }
            Some("server_tool_use") => had_server_tool_use = true,
            Some("web_search_tool_result") => {
                if let Some(items) = block.get("content").and_then(Value::as_array) {
                    for item in items {
                        if item.get("type").and_then(Value::as_str) != Some("web_search_result") {
                            continue;
                        }
                        let url = item
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if url.is_empty() {
                            continue;
                        }
                        let snippet = citations
                            .iter()
                            .find(|(u, _)| u == &url)
                            .map(|(_, c)| c.clone());
                        sources.push(SearchSource {
                            url,
                            title: item.get("title").and_then(Value::as_str).map(str::to_string),
                            page_age: item
                                .get("page_age")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            snippet,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // 按 url 去重（首次保留）
    let mut seen = std::collections::HashSet::new();
    sources.retain(|s| seen.insert(s.url.clone()));

    if sources.is_empty() {
        return Err(ProviderError::new(
            CLASS_NO_RESULT_BLOCK,
            "未返回实际联网结果",
        ));
    }
    Ok(SearchOutcome {
        sources,
        text,
        had_server_tool_use,
    })
}

/// 解析 chat 响应（宽容缺失）。
#[allow(dead_code)] // P2 runner：主 chat 循环消费。
pub fn parse_chat_response(status: u16, body: &str) -> Result<ChatOutcome, ProviderError> {
    classify_status(status)?;
    let value: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::new(CLASS_PARSE, "响应不是有效 JSON"))?;
    if value.get("error").is_some() {
        return Err(ProviderError::new(CLASS_PROVIDER_ERROR, "上游返回错误对象"));
    }
    let content = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::new(CLASS_PARSE, "响应缺少 content"))?;

    let mut text = String::new();
    let mut had_server_tool_use = false;
    let mut had_search_result = false;
    let mut citations = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
                if let Some(cs) = block.get("citations").and_then(Value::as_array) {
                    for c in cs {
                        if let Some(url) = c.get("url").and_then(Value::as_str) {
                            citations.push(url.to_string());
                        }
                    }
                }
            }
            Some("server_tool_use") => had_server_tool_use = true,
            Some("web_search_tool_result") => had_search_result = true,
            Some("tool_use") => {
                let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
                let name = block.get("name").and_then(Value::as_str).unwrap_or_default();
                if !name.is_empty() {
                    tool_calls.push(ToolCall {
                        id: id.to_string(),
                        name: name.to_string(),
                        args: block.get("input").cloned().unwrap_or(Value::Null),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(ChatOutcome {
        text,
        had_server_tool_use,
        had_search_result,
        citations,
        tool_calls,
    })
}

/// 解析 OpenAI 兼容 chat 响应：`choices[0].message.content` + `tool_calls[]`（`arguments` 为
/// JSON 串 → 解析为对象；非法 JSON 保留原串以便诊断）。
#[allow(dead_code)] // feature 构建下由假实现替代；测试仍直接消费。
pub fn parse_openai_chat_response(status: u16, body: &str) -> Result<ChatOutcome, ProviderError> {
    classify_status(status)?;
    let value: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::new(CLASS_PARSE, "响应不是有效 JSON"))?;
    if value.get("error").is_some() {
        return Err(ProviderError::new(CLASS_PROVIDER_ERROR, "上游返回错误对象"));
    }
    let message = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .ok_or_else(|| ProviderError::new(CLASS_PARSE, "响应缺少 choices[0].message"))?;

    let text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let mut tool_calls: Vec<ToolCall> = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let function = call.get("function");
            let name = function
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                continue;
            }
            let arguments = function
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = serde_json::from_str::<Value>(arguments).unwrap_or_else(|_| {
                if arguments.is_empty() {
                    Value::Null
                } else {
                    Value::String(arguments.to_string())
                }
            });
            tool_calls.push(ToolCall { id, name, args });
        }
    }

    Ok(ChatOutcome {
        text,
        had_server_tool_use: false,
        had_search_result: false,
        citations: Vec::new(),
        tool_calls,
    })
}

/// 网络 provider 抽象（Phase 2 runner 复用）；消息 / 工具为**协议无关**表示。
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    #[allow(dead_code)] // P2 runner：主 chat 循环调用。
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        max_tokens: u32,
        cancel: CancelToken,
        deadline: Duration,
    ) -> Result<ChatOutcome, ProviderError>;

    async fn search(
        &self,
        query: &str,
        max_uses: u32,
        cancel: CancelToken,
        deadline: Duration,
    ) -> Result<SearchOutcome, ProviderError>;
}

// ---------------------------------------------------------------------------
// 共享 HTTP 传输（禁重定向 / 30 分钟超时 / 4MiB 上限 / cancel / deadline）
// ---------------------------------------------------------------------------

/// 共享 HTTP 传输：端点 / 认证 / 额外头 / 密钥槽。所有请求都经此，安全不变量集中一处。
#[allow(dead_code)] // feature 构建（agent-test-provider）下工厂改用假实现。
struct Transport {
    client: reqwest::Client,
    base_url: String,
    path: String,
    auth_style: AuthStyle,
    auth_header: String,
    provider_id: String,
    extra_headers: Vec<(String, String)>,
    search_support: bool,
    key_required: bool,
    secret: Arc<dyn SecretStore>,
}

#[allow(dead_code)] // feature 构建下工厂改用假实现。
impl Transport {
    fn from_spec(
        spec: &ResolvedProvider,
        secret: Arc<dyn SecretStore>,
    ) -> Result<Self, ProviderError> {
        // fail closed：构建失败必须向上传播，绝不回退到默认（会跟随重定向、无超时）client。
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| ProviderError::new(CLASS_TRANSPORT, "构建 HTTP 客户端失败"))?;
        Ok(Self {
            client,
            base_url: spec.base_url.clone(),
            path: spec.path.clone(),
            auth_style: spec.auth_style,
            auth_header: spec.auth_header.clone(),
            provider_id: spec.provider_id.clone(),
            extra_headers: spec.extra_headers.clone(),
            search_support: spec.search_support,
            key_required: spec.key_required,
            secret,
        })
    }

    /// 完整端点（`base_url` + `path`）。
    fn endpoint(&self) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), self.path)
    }

    /// 组装请求头（认证 + 额外头）。密钥仅在此读取，绝不回传 / 记录。
    fn headers(&self) -> Result<Vec<(String, String)>, ProviderError> {
        let mut headers = build_auth_headers(
            self.secret.as_ref(),
            &self.provider_id,
            self.auth_style,
            &self.auth_header,
            self.key_required,
        )?;
        for (k, v) in &self.extra_headers {
            headers.push((k.clone(), v.clone()));
        }
        Ok(headers)
    }

    async fn post(
        &self,
        extra: &[(&str, &str)],
        body: &Value,
        cancel: CancelToken,
        deadline: Duration,
    ) -> Result<(u16, String), ProviderError> {
        let mut headers = self.headers()?;
        headers.push(("content-type".to_string(), "application/json".to_string()));
        headers.push(("accept".to_string(), "application/json".to_string()));
        for (k, v) in extra {
            headers.push((k.to_string(), v.to_string()));
        }
        send_request(
            &self.client,
            reqwest::Method::POST,
            &self.endpoint(),
            &headers,
            Some(body),
            cancel,
            deadline,
        )
        .await
    }
}

/// 构造认证头：有密钥 → 按 [`AuthStyle`] 生成；无密钥且 `key_required=false` → 空（keyless）；
/// 无密钥且 `key_required=true` → `MissingCredential`。密钥仅在此读取，绝不回传 / 记录。
pub fn build_auth_headers(
    secret: &dyn SecretStore,
    provider_id: &str,
    auth_style: AuthStyle,
    auth_header: &str,
    key_required: bool,
) -> Result<Vec<(String, String)>, ProviderError> {
    let key = secret
        .get(provider_id)
        .map_err(|_| ProviderError::new(CLASS_MISSING_CREDENTIAL, "读取本机密钥失败"))?;
    match key {
        Some(k) if !k.is_empty() => {
            let value = match auth_style {
                AuthStyle::Bearer => format!("Bearer {}", k.as_str()),
                AuthStyle::ApiKeyHeader => k.as_str().to_string(),
            };
            Ok(vec![(auth_header.to_string(), value)])
        }
        _ if key_required => Err(ProviderError::new(
            CLASS_MISSING_CREDENTIAL,
            "未配置本机密钥",
        )),
        _ => Ok(Vec::new()),
    }
}

/// 只读 GET（模型列表等）：禁重定向、短 deadline、4MiB 上限；非 2xx → 分类错误。
#[allow(dead_code)] // 仅非 feature 实现（真实拉取路径）与测试使用。
pub async fn http_get_json(
    url: &str,
    headers: &[(String, String)],
    deadline: Duration,
) -> Result<String, ProviderError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(deadline)
        .build()
        .map_err(|_| ProviderError::new(CLASS_TRANSPORT, "构建 HTTP 客户端失败"))?;
    let (status, body) = send_request(
        &client,
        reqwest::Method::GET,
        url,
        headers,
        None,
        CancelToken::new(),
        deadline,
    )
    .await?;
    classify_status(status)?;
    Ok(body)
}

/// 共享 HTTP 请求：禁重定向、绝对 deadline（send 与 body 读取共享）、流式累积 + 4MiB 上限、cancel。
async fn send_request(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    headers: &[(String, String)],
    body: Option<&Value>,
    cancel: CancelToken,
    deadline: Duration,
) -> Result<(u16, String), ProviderError> {
    if cancel.is_cancelled() {
        return Err(ProviderError::new(CLASS_ABORTED, "已取消"));
    }
    let deadline_at = tokio::time::Instant::now() + deadline;
    let mut request = client.request(method, url).timeout(deadline);
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }
    if let Some(body) = body {
        request = request.json(body);
    }

    let mut resp = tokio::select! {
        _ = wait_cancel(cancel.clone()) => {
            return Err(ProviderError::new(CLASS_ABORTED, "已取消"));
        }
        r = tokio::time::timeout_at(deadline_at, request.send()) => match r {
            Err(_) => return Err(ProviderError::new(CLASS_ABORTED, "已超时")),
            Ok(Err(e)) => return Err(map_transport(e, "网络请求失败")),
            Ok(Ok(resp)) => resp,
        },
    };

    let status = resp.status().as_u16();
    // 预检 Content-Length（若存在）；无则靠下方流式累积即时限额拦截。
    if resp
        .content_length()
        .is_some_and(|len| len as usize > MAX_RESPONSE_BYTES)
    {
        return Err(ProviderError::new(CLASS_PARSE, "响应体超出限额"));
    }

    // 流式累积：受 cancel / deadline 约束；超 MAX_RESPONSE_BYTES 立即中止，不整包缓冲。
    let mut body_bytes: Vec<u8> = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = wait_cancel(cancel.clone()) => {
                return Err(ProviderError::new(CLASS_ABORTED, "已取消"));
            }
            r = tokio::time::timeout_at(deadline_at, resp.chunk()) => match r {
                Err(_) => return Err(ProviderError::new(CLASS_ABORTED, "已超时")),
                Ok(Err(e)) => return Err(map_transport(e, "读取响应失败")),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(chunk))) => chunk,
            },
        };
        if body_bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(ProviderError::new(CLASS_PARSE, "响应体超出限额"));
        }
        body_bytes.extend_from_slice(&chunk);
    }
    Ok((status, String::from_utf8_lossy(&body_bytes).to_string()))
}

/// reqwest 错误 → ProviderError；超时一律归 `Aborted`（deadline），其余 `Transport`。
fn map_transport(err: reqwest::Error, context: &'static str) -> ProviderError {
    if err.is_timeout() {
        ProviderError::new(CLASS_ABORTED, "已超时")
    } else {
        ProviderError::new(CLASS_TRANSPORT, context)
    }
}

// ---------------------------------------------------------------------------
// Anthropic Messages 兼容适配器
// ---------------------------------------------------------------------------

/// Anthropic 兼容协议 provider；端点 / 认证 / 凭据槽取自 [`ResolvedProvider`]（见 ADR-0009）。
#[allow(dead_code)] // feature 构建（agent-test-provider）下工厂改用假实现，本实现仅供默认 / release。
pub struct AnthropicCompatProvider {
    transport: Transport,
    model: String,
    search_system_hint: Option<String>,
}

#[allow(dead_code)] // feature 构建下工厂改用假实现。
impl AnthropicCompatProvider {
    /// 按已解析规格构造；禁跟随重定向；client 构建失败向上传播（fail closed）。
    pub fn from_spec(
        spec: &ResolvedProvider,
        secret: Arc<dyn SecretStore>,
        model: String,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            transport: Transport::from_spec(spec, secret)?,
            model,
            search_system_hint: spec.search_system_hint.clone(),
        })
    }
}

#[async_trait::async_trait]
impl Provider for AnthropicCompatProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        max_tokens: u32,
        cancel: CancelToken,
        deadline: Duration,
    ) -> Result<ChatOutcome, ProviderError> {
        let body = build_chat_request(
            &self.model,
            anthropic_messages(messages),
            anthropic_tools(tools),
            max_tokens,
        );
        let (status, text) = self
            .transport
            .post(
                &[("anthropic-version", ANTHROPIC_VERSION)],
                &body,
                cancel,
                deadline,
            )
            .await?;
        parse_chat_response(status, &text)
    }

    async fn search(
        &self,
        query: &str,
        max_uses: u32,
        cancel: CancelToken,
        deadline: Duration,
    ) -> Result<SearchOutcome, ProviderError> {
        if !self.transport.search_support {
            return Err(ProviderError::new(
                CLASS_CONFIG,
                "该 provider 未声明支持服务端搜索",
            ));
        }
        let body = build_search_request(
            &self.model,
            query,
            max_uses,
            SEARCH_MAX_TOKENS,
            self.search_system_hint.as_deref(),
        );
        let (status, text) = self
            .transport
            .post(
                &[("anthropic-version", ANTHROPIC_VERSION)],
                &body,
                cancel,
                deadline,
            )
            .await?;
        parse_search_response(status, &text)
    }
}

// ---------------------------------------------------------------------------
// OpenAI Chat Completions 兼容适配器
// ---------------------------------------------------------------------------

/// OpenAI 兼容协议 provider；端点 / 认证 / 凭据槽取自 [`ResolvedProvider`]（见 ADR-0009）。
#[allow(dead_code)] // feature 构建（agent-test-provider）下工厂改用假实现，本实现仅供默认 / release。
pub struct OpenAiCompatProvider {
    transport: Transport,
    model: String,
}

#[allow(dead_code)] // feature 构建下工厂改用假实现。
impl OpenAiCompatProvider {
    /// 按已解析规格构造；禁跟随重定向；client 构建失败向上传播（fail closed）。
    pub fn from_spec(
        spec: &ResolvedProvider,
        secret: Arc<dyn SecretStore>,
        model: String,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            transport: Transport::from_spec(spec, secret)?,
            model,
        })
    }
}

#[async_trait::async_trait]
impl Provider for OpenAiCompatProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        max_tokens: u32,
        cancel: CancelToken,
        deadline: Duration,
    ) -> Result<ChatOutcome, ProviderError> {
        let body = build_openai_chat_request(
            &self.model,
            openai_messages(messages),
            openai_tools(tools),
            max_tokens,
        );
        let (status, text) = self.transport.post(&[], &body, cancel, deadline).await?;
        parse_openai_chat_response(status, &text)
    }

    async fn search(
        &self,
        _query: &str,
        _max_uses: u32,
        _cancel: CancelToken,
        _deadline: Duration,
    ) -> Result<SearchOutcome, ProviderError> {
        // OpenAI 兼容协议暂无服务端搜索（不引入第三方搜索服务）；fail closed。
        Err(ProviderError::new(
            CLASS_CONFIG,
            "OpenAI 兼容协议未实现服务端搜索",
        ))
    }
}

/// 测试用假 provider（仅替换网络，不绕过任何安全规则）；`#[cfg]` 门禁。
#[cfg(any(test, feature = "agent-test-provider"))]
#[allow(dead_code)] // 假实现字段供 P2 / 集成测试扩展。
pub struct FakeProvider {
    pub search: std::sync::Mutex<Result<SearchOutcome, ProviderError>>,
    pub chat: std::sync::Mutex<Result<ChatOutcome, ProviderError>>,
    /// 可选多步脚本：非空时每次 `chat` 弹出队首，用于主循环测试。
    pub chat_script: std::sync::Mutex<std::collections::VecDeque<Result<ChatOutcome, ProviderError>>>,
    /// 可选搜索脚本：非空时每次 `search` 弹出队首。
    pub search_script: std::sync::Mutex<std::collections::VecDeque<Result<SearchOutcome, ProviderError>>>,
    /// 为真时 `chat` 反复返回脚本队首（不弹出）——用于驱动步数上限。
    pub repeat_last_chat: bool,
    pub delay: Duration,
}

#[cfg(any(test, feature = "agent-test-provider"))]
impl FakeProvider {
    pub fn search_ok(outcome: SearchOutcome) -> Self {
        Self {
            search: std::sync::Mutex::new(Ok(outcome)),
            chat: std::sync::Mutex::new(Ok(ChatOutcome {
                text: String::new(),
                had_server_tool_use: false,
                had_search_result: false,
                citations: Vec::new(),
                tool_calls: Vec::new(),
            })),
            chat_script: std::sync::Mutex::new(std::collections::VecDeque::new()),
            search_script: std::sync::Mutex::new(std::collections::VecDeque::new()),
            repeat_last_chat: false,
            delay: Duration::ZERO,
        }
    }

    /// 主循环脚本：按序返回 `chat` 结果；`search` 用给定结果。
    #[allow(dead_code)] // feature 构建下仅测试消费。
    pub fn scripted(
        chat_steps: Vec<Result<ChatOutcome, ProviderError>>,
        search: Result<SearchOutcome, ProviderError>,
    ) -> Self {
        let mut provider = Self::search_ok(SearchOutcome {
            sources: Vec::new(),
            text: String::new(),
            had_server_tool_use: false,
        });
        provider.search = std::sync::Mutex::new(search);
        provider.chat_script = std::sync::Mutex::new(chat_steps.into());
        provider
    }
}

#[cfg(any(test, feature = "agent-test-provider"))]
#[async_trait::async_trait]
impl Provider for FakeProvider {
    async fn chat(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
        _max_tokens: u32,
        cancel: CancelToken,
        _deadline: Duration,
    ) -> Result<ChatOutcome, ProviderError> {
        if self.delay > Duration::ZERO {
            tokio::select! {
                _ = wait_cancel(cancel) => return Err(ProviderError::new(CLASS_ABORTED, "已取消")),
                _ = tokio::time::sleep(self.delay) => {}
            }
        }
        {
            let mut script = self.chat_script.lock().unwrap();
            if self.repeat_last_chat {
                if let Some(last) = script.front() {
                    return last.clone();
                }
            } else if let Some(next) = script.pop_front() {
                return next;
            }
        }
        self.chat.lock().unwrap().clone()
    }

    async fn search(
        &self,
        _query: &str,
        _max_uses: u32,
        cancel: CancelToken,
        _deadline: Duration,
    ) -> Result<SearchOutcome, ProviderError> {
        if self.delay > Duration::ZERO {
            tokio::select! {
                _ = wait_cancel(cancel) => return Err(ProviderError::new(CLASS_ABORTED, "已取消")),
                _ = tokio::time::sleep(self.delay) => {}
            }
        }
        if let Some(next) = self.search_script.lock().unwrap().pop_front() {
            return next;
        }
        self.search.lock().unwrap().clone()
    }
}

// ---------------------------------------------------------------------------
// 脚本化假 provider（fixture 驱动）+ 编译期 feature 门禁的统一工厂
// ---------------------------------------------------------------------------

/// 内置默认脚本（`agent-test-provider` 构建下 `make_provider` 使用）：仅 native_search + 终答，
/// 不含 fs 路径（授权根由任务运行时提供），完全离线。供 CDP 全链路验证。
#[cfg(feature = "agent-test-provider")]
pub const DEFAULT_TEST_SCRIPT: &str =
    include_str!("../../tests/fixtures/agent/default_success.json");

#[cfg(any(test, feature = "agent-test-provider"))]
#[derive(serde::Deserialize)]
struct ScriptFile {
    #[serde(default)]
    delay_ms: u64,
    #[serde(default)]
    repeat_last_chat: bool,
    #[serde(default)]
    chat: Vec<ScriptChatStep>,
    #[serde(default)]
    search: Vec<ScriptSearchStep>,
}

#[cfg(any(test, feature = "agent-test-provider"))]
#[derive(serde::Deserialize)]
struct ScriptChatStep {
    #[serde(default)]
    text: String,
    #[serde(default)]
    tool_calls: Vec<ScriptToolCall>,
    #[serde(default)]
    error_class: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
}

#[cfg(any(test, feature = "agent-test-provider"))]
impl ScriptChatStep {
    fn into_result(self) -> Result<ChatOutcome, ProviderError> {
        if let Some(class) = self.error_class {
            return Err(ProviderError::new(
                class_name_to_static(&class),
                self.error_message.unwrap_or_default(),
            ));
        }
        Ok(ChatOutcome {
            text: self.text,
            had_server_tool_use: false,
            had_search_result: false,
            citations: Vec::new(),
            tool_calls: self
                .tool_calls
                .into_iter()
                .map(|t| ToolCall {
                    id: t.id,
                    name: t.name,
                    args: t.input,
                })
                .collect(),
        })
    }
}

#[cfg(any(test, feature = "agent-test-provider"))]
#[derive(serde::Deserialize)]
struct ScriptToolCall {
    #[serde(default)]
    id: String,
    name: String,
    #[serde(default)]
    input: Value,
}

#[cfg(any(test, feature = "agent-test-provider"))]
#[derive(serde::Deserialize)]
struct ScriptSearchStep {
    #[serde(default)]
    sources: Vec<ScriptSource>,
    #[serde(default)]
    error_class: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
}

#[cfg(any(test, feature = "agent-test-provider"))]
impl ScriptSearchStep {
    fn into_result(self) -> Result<SearchOutcome, ProviderError> {
        if let Some(class) = self.error_class {
            return Err(ProviderError::new(
                class_name_to_static(&class),
                self.error_message.unwrap_or_default(),
            ));
        }
        Ok(SearchOutcome {
            sources: self
                .sources
                .into_iter()
                .map(|s| SearchSource {
                    url: s.url,
                    title: s.title,
                    page_age: s.page_age,
                    snippet: s.snippet,
                })
                .collect(),
            text: String::new(),
            had_server_tool_use: false,
        })
    }
}

#[cfg(any(test, feature = "agent-test-provider"))]
#[derive(serde::Deserialize)]
struct ScriptSource {
    url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    page_age: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
}

/// 从 fixture 文件加载脚本（`vars` 为 JSON 字符串内的占位替换，如 `("$ROOT", 绝对路径)`）。
///
/// 路径可注入；**不触网**。占位值按 JSON 字符串转义后替换。
#[cfg(any(test, feature = "agent-test-provider"))]
#[allow(dead_code)] // 供集成测试 / CDP 注入脚本路径。
pub fn load_fake_script(
    path: &std::path::Path,
    vars: &[(&str, &str)],
) -> Result<FakeProvider, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取脚本失败：{e}"))?;
    load_fake_script_str(&text, vars)
}

/// 从脚本字符串加载（`make_provider` 内置脚本 / 单元测试用）。
#[cfg(any(test, feature = "agent-test-provider"))]
#[allow(dead_code)]
pub fn load_fake_script_str(text: &str, vars: &[(&str, &str)]) -> Result<FakeProvider, String> {
    let mut rendered = text.to_string();
    for (key, value) in vars {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        rendered = rendered.replace(key, &escaped);
    }
    let file: ScriptFile =
        serde_json::from_str(&rendered).map_err(|e| format!("解析脚本失败：{e}"))?;

    let chat: Vec<Result<ChatOutcome, ProviderError>> =
        file.chat.into_iter().map(ScriptChatStep::into_result).collect();
    let search: Vec<Result<SearchOutcome, ProviderError>> = file
        .search
        .into_iter()
        .map(ScriptSearchStep::into_result)
        .collect();

    let mut provider = FakeProvider::search_ok(SearchOutcome {
        sources: Vec::new(),
        text: String::new(),
        had_server_tool_use: false,
    });
    provider.chat_script = std::sync::Mutex::new(chat.into());
    provider.search_script = std::sync::Mutex::new(search.into());
    provider.delay = Duration::from_millis(file.delay_ms);
    provider.repeat_last_chat = file.repeat_last_chat;
    Ok(provider)
}

#[cfg(feature = "agent-test-provider")]
impl FakeProvider {
    /// `agent-test-provider` 构建下的工厂默认假实现（内置离线脚本）。
    pub fn from_default_script() -> Self {
        load_fake_script_str(DEFAULT_TEST_SCRIPT, &[]).expect("内置测试脚本应可解析")
    }
}

/// 统一 provider 工厂（编译期 feature 门禁）。
///
/// - `agent-test-provider` **开启**：返回离线 [`FakeProvider`]（**只替换网络**，不绕过沙箱 /
///   路径 / 预算 / 密钥规则）。
/// - 默认 / **release（feature 关闭）**：按 [`ResolvedProvider`] 的协议返回
///   [`AnthropicCompatProvider`] 或 [`OpenAiCompatProvider`]（预设端点，fail-closed）。
///
/// **不是运行时环境变量开关**：分支由 `#[cfg(feature = ...)]` 在编译期决定，release 二进制
/// 不包含假实现（见 `agent-test-provider` feature 定义与 `TEST_PROVIDER_ENABLED`）。
#[cfg(feature = "agent-test-provider")]
pub fn make_provider(
    _spec: &ResolvedProvider,
    _secret: Arc<dyn SecretStore>,
    _model: String,
) -> Result<Arc<dyn Provider>, ProviderError> {
    Ok(Arc::new(FakeProvider::from_default_script()))
}

#[cfg(not(feature = "agent-test-provider"))]
pub fn make_provider(
    spec: &ResolvedProvider,
    secret: Arc<dyn SecretStore>,
    model: String,
) -> Result<Arc<dyn Provider>, ProviderError> {
    match spec.protocol {
        super::presets::Protocol::AnthropicCompat => Ok(Arc::new(
            AnthropicCompatProvider::from_spec(spec, secret, model)?,
        )),
        super::presets::Protocol::OpenAiCompat => Ok(Arc::new(OpenAiCompatProvider::from_spec(
            spec, secret, model,
        )?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::presets::Protocol;

    const OK_BODY: &str = r#"{
      "content": [
        { "type": "text", "text": "Here is what I found.",
          "citations": [ { "type": "web_search_result_location", "url": "https://a.test", "cited_text": "excerpt for A" } ] },
        { "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search" },
        { "type": "web_search_tool_result",
          "content": [
            { "type": "web_search_result", "url": "https://a.test", "title": "A", "page_age": "2026-02-02" },
            { "type": "web_search_result", "url": "https://b.test", "title": "B" }
          ] }
      ]
    }"#;

    #[test]
    fn search_request_shape_is_server_tool_only() {
        let body = build_search_request(
            "deepseek-flash",
            "DeepSeek API docs",
            5,
            SEARCH_MAX_TOKENS,
            None,
        );
        assert_eq!(body["model"], "deepseek-flash");
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], SEARCH_TOOL_TYPE);
        assert_eq!(tools[0]["name"], SEARCH_TOOL_NAME);
        assert_eq!(tools[0]["max_uses"], 5);
        // 不含任何客户端 fs 工具
        assert!(tools[0].get("input_schema").is_none());
    }

    #[test]
    fn chat_request_keeps_client_tools_separate() {
        let tools = json!([{ "name": "fs_read", "input_schema": { "type": "object" } }]);
        let body = build_chat_request("m", json!([]), tools, CHAT_MAX_TOKENS);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "fs_read");
        // 主 chat 不应声明服务端 web_search
        assert!(tools.iter().all(|t| t["type"] != SEARCH_TOOL_TYPE));
    }

    #[test]
    fn chat_request_omits_empty_tools() {
        // tools 为空 → 不发送 tools 字段（与 OpenAI 对齐）
        let body = build_chat_request("m", json!([]), json!([]), 16);
        assert!(body.get("tools").is_none());
        assert_eq!(body["model"], "m");
        assert_eq!(body["max_tokens"], 16);
        // 非空 → 保留
        let tools = json!([{ "name": "fs_read", "input_schema": { "type": "object" } }]);
        let body = build_chat_request("m", json!([]), tools, 16);
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn parse_search_extracts_sources_and_snippets() {
        let out = parse_search_response(200, OK_BODY).unwrap();
        assert_eq!(out.sources.len(), 2);
        assert_eq!(out.sources[0].url, "https://a.test");
        assert_eq!(out.sources[0].page_age.as_deref(), Some("2026-02-02"));
        assert_eq!(out.sources[0].snippet.as_deref(), Some("excerpt for A"));
        assert_eq!(out.sources[1].snippet, None, "未被引用的结果无 snippet");
        assert!(out.had_server_tool_use);
    }

    #[test]
    fn parse_search_fails_without_result_or_when_empty() {
        // 无 web_search_tool_result 块
        let no_block = r#"{ "content": [ { "type": "text", "text": "I could not find results." } ] }"#;
        assert_eq!(
            parse_search_response(200, no_block).unwrap_err().class(),
            CLASS_NO_RESULT_BLOCK
        );
        // 有块但无实际来源
        let empty = r#"{ "content": [ { "type": "web_search_tool_result", "content": [] } ] }"#;
        assert_eq!(
            parse_search_response(200, empty).unwrap_err().class(),
            CLASS_NO_RESULT_BLOCK
        );
    }

    #[test]
    fn parse_search_classifies_http_errors() {
        assert_eq!(
            parse_search_response(302, "{}").unwrap_err().class(),
            CLASS_REDIRECT
        );
        assert_eq!(
            parse_search_response(401, "{}").unwrap_err().class(),
            CLASS_HTTP_AUTH
        );
        assert_eq!(
            parse_search_response(503, "{}").unwrap_err().class(),
            CLASS_HTTP_STATUS
        );
        assert_eq!(
            parse_search_response(200, "not json").unwrap_err().class(),
            CLASS_PARSE
        );
        // 上游错误对象 → 单列 ProviderError（区别于 HttpStatus）
        assert_eq!(
            parse_search_response(200, r#"{"error":{"message":"boom"}}"#)
                .unwrap_err()
                .class(),
            CLASS_PROVIDER_ERROR
        );
        assert_eq!(
            parse_chat_response(200, r#"{"error":"boom"}"#)
                .unwrap_err()
                .class(),
            CLASS_PROVIDER_ERROR
        );
    }

    #[test]
    fn parse_chat_tolerates_missing_citations() {
        let plain = r#"{ "content": [ { "type": "text", "text": "hi" } ] }"#;
        let out = parse_chat_response(200, plain).unwrap();
        assert_eq!(out.text, "hi");
        assert!(out.citations.is_empty());
        assert!(!out.had_server_tool_use);
    }

    #[test]
    fn error_messages_do_not_leak_body() {
        let secret_marker = "sk-SUPER-SECRET";
        let err = parse_search_response(200, &format!(r#"{{"error":"{secret_marker}","x":1}}"#))
            .unwrap_err();
        assert!(!err.message().contains(secret_marker));
    }

    #[tokio::test]
    async fn fake_provider_search_returns_sources() {
        let outcome = parse_search_response(200, OK_BODY).unwrap();
        let provider = FakeProvider::search_ok(outcome.clone());
        let got = provider
            .search("q", 5, CancelToken::new(), Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(got, outcome);
    }

    #[tokio::test]
    async fn fake_provider_respects_cancellation() {
        let provider = FakeProvider {
            search: std::sync::Mutex::new(Err(ProviderError::new(CLASS_NO_RESULT_BLOCK, "x"))),
            chat: std::sync::Mutex::new(Err(ProviderError::new(CLASS_NO_RESULT_BLOCK, "x"))),
            chat_script: std::sync::Mutex::new(std::collections::VecDeque::new()),
            search_script: std::sync::Mutex::new(std::collections::VecDeque::new()),
            repeat_last_chat: false,
            delay: Duration::from_secs(5),
        };
        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            c2.cancel();
        });
        let err = provider
            .search("q", 1, cancel, Duration::from_secs(30))
            .await
            .unwrap_err();
        assert_eq!(err.class(), CLASS_ABORTED);
    }

    // ---- 本地离线 mock server：真实 HTTP 路径（不连真实网络）----

    use std::net::TcpStream;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc as StdArc;
    use std::thread;

    fn read_request(s: &mut TcpStream) {
        use std::io::Read;
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buf = [0u8; 4096];
        let mut total: Vec<u8> = Vec::new();
        while !total.windows(4).any(|w| w == b"\r\n\r\n") {
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => total.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
    }

    /// 绑定 127.0.0.1 随机端口；返回端口与请求计数。responder 每连接调用一次。
    fn spawn_mock<F>(responder: F) -> (u16, StdArc<AtomicUsize>)
    where
        F: Fn(&mut TcpStream) + Send + 'static,
    {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let count = StdArc::new(AtomicUsize::new(0));
        let c = count.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { break };
                c.fetch_add(1, Ordering::SeqCst);
                responder(&mut s);
            }
        });
        (port, count)
    }

    fn provider_at(port: u16) -> AnthropicCompatProvider {
        let store: Arc<dyn SecretStore> =
            Arc::new(crate::agent::secret::FakeSecretStore::new(Some("test-key")));
        let spec = ResolvedProvider {
            provider_id: "deepseek".to_string(),
            protocol: Protocol::AnthropicCompat,
            base_url: format!("http://127.0.0.1:{port}/anthropic/v1"),
            path: "/messages".to_string(),
            auth_style: AuthStyle::ApiKeyHeader,
            auth_header: "x-api-key".to_string(),
            extra_headers: Vec::new(),
            search_support: true,
            search_system_hint: None,
            key_required: true,
        };
        AnthropicCompatProvider::from_spec(&spec, store, "deepseek-flash".to_string()).unwrap()
    }

    /// 启动捕获请求 + 固定响应的 mock；返回端口与捕获到的原始请求。
    fn spawn_capture(body: &'static str) -> (u16, StdArc<std::sync::Mutex<String>>) {
        let captured = StdArc::new(std::sync::Mutex::new(String::new()));
        let cap = captured.clone();
        let (port, _count) = spawn_mock(move |s| {
            use std::io::{Read, Write};
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 8192];
            let mut total: Vec<u8> = Vec::new();
            while !total.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => total.extend_from_slice(&buf[..n]),
                    Err(_) => break,
                }
            }
            *cap.lock().unwrap() = String::from_utf8_lossy(&total).to_string();
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.flush();
        });
        (port, captured)
    }

    #[tokio::test]
    async fn redirect_is_not_followed_and_only_one_request() {
        use std::io::Write;
        let (port, count) = spawn_mock(|s| {
            read_request(s);
            let _ = s.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            let _ = s.flush();
        });
        let provider = provider_at(port);
        let err = provider
            .search("q", 1, CancelToken::new(), Duration::from_secs(5))
            .await
            .unwrap_err();
        assert_eq!(err.class(), CLASS_REDIRECT);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "302 不得跟随 → 仅一次请求");
    }

    #[tokio::test]
    async fn oversized_chunked_body_without_content_length_is_rejected() {
        use std::io::Write;
        let (port, _count) = spawn_mock(|s| {
            read_request(s);
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
            let chunk = vec![b'a'; 1024 * 1024];
            for _ in 0..5 {
                // 5 × 1MiB > MAX_RESPONSE_BYTES(4MiB)，且无 Content-Length
                if s.write_all(format!("{:x}\r\n", chunk.len()).as_bytes()).is_err() {
                    return;
                }
                if s.write_all(&chunk).is_err() || s.write_all(b"\r\n").is_err() {
                    return;
                }
            }
            let _ = s.write_all(b"0\r\n\r\n");
            let _ = s.flush();
        });
        let provider = provider_at(port);
        let err = provider
            .search("q", 1, CancelToken::new(), Duration::from_secs(30))
            .await
            .unwrap_err();
        assert_eq!(err.class(), CLASS_PARSE, "chunked 超限应即时失败");
        assert!(err.message().contains("限额"));
    }

    #[tokio::test]
    async fn deadline_interrupts_stalled_body() {
        use std::io::Write;
        let (port, _count) = spawn_mock(|s| {
            read_request(s);
            let _ = s.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n",
            );
            let _ = s.write_all(b"partial");
            let _ = s.flush();
            thread::sleep(Duration::from_secs(3)); // 头到达后停滞
        });
        let provider = provider_at(port);
        let start = std::time::Instant::now();
        let err = provider
            .search("q", 1, CancelToken::new(), Duration::from_millis(300))
            .await
            .unwrap_err();
        assert_eq!(err.class(), CLASS_ABORTED, "停滞 body 应被 deadline 中断");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn cancel_during_body_returns_aborted() {
        use std::io::Write;
        let (port, _count) = spawn_mock(|s| {
            read_request(s);
            let _ = s.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n",
            );
            let _ = s.write_all(b"partial");
            let _ = s.flush();
            thread::sleep(Duration::from_secs(3));
        });
        let provider = provider_at(port);
        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            c2.cancel();
        });
        let start = std::time::Instant::now();
        let err = provider
            .search("q", 1, cancel, Duration::from_secs(30))
            .await
            .unwrap_err();
        assert_eq!(err.class(), CLASS_ABORTED);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn factory_is_compile_time_feature_gated() {
        // 编译期事实与 feature 一致（经局部变量避免常量断言 lint）
        let enabled = TEST_PROVIDER_ENABLED;
        assert_eq!(enabled, cfg!(feature = "agent-test-provider"));

        #[cfg(feature = "agent-test-provider")]
        {
            // feature 开：工厂返回离线假实现（无真实网络也可返回脚本来源）
            let store: Arc<dyn SecretStore> =
                Arc::new(crate::agent::secret::FakeSecretStore::new(Some("k")));
            let preset = crate::agent::presets::find("deepseek").unwrap();
            let spec = crate::agent::presets::resolve_provider(preset, "deepseek-flash").unwrap();
            let p = make_provider(&spec, store, "m".to_string()).unwrap();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let out = rt
                .block_on(p.search("q", 1, CancelToken::new(), Duration::from_secs(1)))
                .unwrap();
            assert!(!out.sources.is_empty(), "feature 开 → 工厂返回离线假实现");
        }

        #[cfg(not(feature = "agent-test-provider"))]
        {
            // 默认构建证明：`make_provider` 走 `#[cfg(not(feature))]` 定义（真实 AnthropicCompatProvider），
            // 假实现分支未被编译进该定义。
            assert!(!enabled, "默认构建不得返回假实现");
        }
    }

    #[test]
    fn preset_resolves_to_endpoint_and_auth() {
        use crate::agent::presets;
        let ds = presets::find("deepseek").unwrap();
        let spec = presets::resolve_provider(ds, "deepseek-flash").unwrap();
        let store: Arc<dyn SecretStore> =
            Arc::new(crate::agent::secret::FakeSecretStore::new(Some("k")));
        let p = AnthropicCompatProvider::from_spec(&spec, store, "deepseek-flash".into()).unwrap();
        assert_eq!(
            p.transport.endpoint(),
            "https://api.deepseek.com/anthropic/v1/messages",
            "默认 deepseek 端点须与旧实现一致"
        );
        assert_eq!(p.transport.auth_header, "x-api-key");
        assert_eq!(p.transport.provider_id, "deepseek");

        // OpenAI 预设 → OpenAI 适配器端点 / Bearer
        let oa = presets::find("openai").unwrap();
        let spec_oa = presets::resolve_provider(oa, "gpt-5.6-sol").unwrap();
        let store_oa: Arc<dyn SecretStore> =
            Arc::new(crate::agent::secret::FakeSecretStore::with_slot("openai", "sk-oa"));
        let p_oa =
            OpenAiCompatProvider::from_spec(&spec_oa, store_oa, "gpt-5.6-sol".into()).unwrap();
        assert_eq!(
            p_oa.transport.endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(p_oa.transport.auth_style, AuthStyle::Bearer);
        assert_eq!(p_oa.transport.provider_id, "openai");
    }

    #[tokio::test]
    async fn request_uses_preset_path_and_auth_header() {
        let body = r#"{"content":[{"type":"web_search_tool_result","content":[{"type":"web_search_result","url":"https://x.test","title":"X"}]}]}"#;
        let (port, captured) = spawn_capture(body);
        let store: Arc<dyn SecretStore> = Arc::new(
            crate::agent::secret::FakeSecretStore::with_slot("anthropic", "sk-ant-test"),
        );
        let spec = ResolvedProvider {
            provider_id: "anthropic".to_string(),
            protocol: Protocol::AnthropicCompat,
            base_url: format!("http://127.0.0.1:{port}/v1"),
            path: "/messages".to_string(),
            auth_style: AuthStyle::ApiKeyHeader,
            auth_header: "x-api-key".to_string(),
            extra_headers: Vec::new(),
            search_support: true,
            search_system_hint: None,
            key_required: true,
        };
        let provider =
            AnthropicCompatProvider::from_spec(&spec, store, "claude-x".to_string()).unwrap();
        provider
            .search("q", 1, CancelToken::new(), Duration::from_secs(5))
            .await
            .unwrap();
        let req = captured.lock().unwrap().clone();
        assert!(
            req.starts_with("POST /v1/messages"),
            "请求路径应含 messages 端点：{req}"
        );
        assert!(
            req.to_lowercase().contains("x-api-key: sk-ant-test"),
            "应携带预设认证头与对应槽密钥：{req}"
        );
    }

    // ---- 协议转换（纯函数）----

    fn sample_conversation() -> Vec<ChatMessage> {
        vec![
            ChatMessage::User("研究一下".to_string()),
            ChatMessage::Assistant {
                text: "先读文件".to_string(),
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "fs_read".to_string(),
                    args: json!({ "path": "D:\\a.txt" }),
                }],
            },
            ChatMessage::ToolResults(vec![ToolResult {
                id: "call_1".to_string(),
                content: "hello".to_string(),
                is_error: false,
            }]),
        ]
    }

    fn sample_tools() -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: "fs_read".to_string(),
            description: "读取".to_string(),
            input_schema: json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
        }]
    }

    #[test]
    fn anthropic_and_openai_wire_carry_same_semantics() {
        let conv = sample_conversation();
        let tools = sample_tools();

        let a_msgs = anthropic_messages(&conv);
        assert_eq!(a_msgs[1]["content"][1]["type"], "tool_use");
        assert_eq!(a_msgs[1]["content"][1]["input"]["path"], "D:\\a.txt");
        assert_eq!(a_msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(a_msgs[2]["content"][0]["tool_use_id"], "call_1");
        let a_tools = anthropic_tools(&tools);
        assert_eq!(a_tools[0]["input_schema"]["type"], "object");

        let o_msgs = openai_messages(&conv);
        assert_eq!(o_msgs[1]["tool_calls"][0]["function"]["name"], "fs_read");
        // arguments 为 JSON 串
        let args: Value =
            serde_json::from_str(o_msgs[1]["tool_calls"][0]["function"]["arguments"].as_str().unwrap())
                .unwrap();
        assert_eq!(args["path"], "D:\\a.txt");
        // 工具结果 → role:"tool"
        assert_eq!(o_msgs[2]["role"], "tool");
        assert_eq!(o_msgs[2]["tool_call_id"], "call_1");
        let o_tools = openai_tools(&tools);
        assert_eq!(o_tools[0]["type"], "function");
        assert_eq!(o_tools[0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn openai_request_shape_and_empty_tools() {
        let body = build_openai_chat_request(
            "gpt-5.6-sol",
            openai_messages(&sample_conversation()),
            openai_tools(&sample_tools()),
            CHAT_MAX_TOKENS,
        );
        assert_eq!(body["model"], "gpt-5.6-sol");
        assert_eq!(body["max_tokens"], CHAT_MAX_TOKENS);
        assert_eq!(body["tools"][0]["type"], "function");
        // 空 tools 不发送 tools 字段
        let bare = build_openai_chat_request("m", json!([]), json!([]), 16);
        assert!(bare.get("tools").is_none());
    }

    #[test]
    fn parse_openai_extracts_text_and_tool_calls() {
        let body = r#"{
          "choices": [ { "message": {
            "content": "ok",
            "tool_calls": [
              { "id": "call_a", "type": "function",
                "function": { "name": "native_search", "arguments": "{\"query\":\"q\"}" } }
            ]
          } } ]
        }"#;
        let out = parse_openai_chat_response(200, body).unwrap();
        assert_eq!(out.text, "ok");
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].id, "call_a");
        assert_eq!(out.tool_calls[0].name, "native_search");
        assert_eq!(out.tool_calls[0].args["query"], "q");
        assert!(!out.had_server_tool_use);
        assert!(out.citations.is_empty());
    }

    #[test]
    fn parse_openai_invalid_arguments_kept_as_string() {
        let body = r#"{"choices":[{"message":{"content":"","tool_calls":[
          {"id":"c","function":{"name":"fs_read","arguments":"not-json"}}]}}]}"#;
        let out = parse_openai_chat_response(200, body).unwrap();
        assert_eq!(out.tool_calls[0].args, Value::String("not-json".to_string()));
    }

    #[test]
    fn parse_openai_classifies_errors() {
        assert_eq!(
            parse_openai_chat_response(302, "{}").unwrap_err().class(),
            CLASS_REDIRECT
        );
        assert_eq!(
            parse_openai_chat_response(401, "{}").unwrap_err().class(),
            CLASS_HTTP_AUTH
        );
        assert_eq!(
            parse_openai_chat_response(200, r#"{"error":{"message":"boom"}}"#)
                .unwrap_err()
                .class(),
            CLASS_PROVIDER_ERROR
        );
        assert_eq!(
            parse_openai_chat_response(200, r#"{"choices":[]}"#)
                .unwrap_err()
                .class(),
            CLASS_PARSE
        );
    }

    #[test]
    fn both_protocols_parse_equivalent_tool_calls_to_same_neutral() {
        let anthropic_body = r#"{"content":[{"type":"tool_use","id":"c1","name":"fs_read","input":{"path":"D:\\a.txt"}}]}"#;
        let openai_body = r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"fs_read","arguments":"{\"path\":\"D:\\\\a.txt\"}"}}]}}]}"#;
        let a = parse_chat_response(200, anthropic_body).unwrap();
        let o = parse_openai_chat_response(200, openai_body).unwrap();
        assert_eq!(a.tool_calls, o.tool_calls, "双协议应产出同一协议无关表示");
        assert_eq!(a.tool_calls[0].args["path"], "D:\\a.txt");
    }

    #[test]
    fn qwen_search_request_injects_system_hint() {
        let hint = "x-anthropic-billing-header: cc_entrypoint=cli;";
        let body = build_search_request("qwen3.8-max", "q", 3, SEARCH_MAX_TOKENS, Some(hint));
        assert_eq!(body["system"][0]["type"], "text");
        assert_eq!(body["system"][0]["text"], hint);
        // 无 hint → 不注入 system
        let bare = build_search_request("deepseek-flash", "q", 3, SEARCH_MAX_TOKENS, None);
        assert!(bare.get("system").is_none());
    }

    #[tokio::test]
    async fn openai_request_sends_bearer_and_extra_headers() {
        let body = r#"{"choices":[{"message":{"content":"hi"}}]}"#;
        let (port, captured) = spawn_capture(body);
        let store: Arc<dyn SecretStore> =
            Arc::new(crate::agent::secret::FakeSecretStore::with_slot("opencode-go", "sk-go"));
        let spec = ResolvedProvider {
            provider_id: "opencode-go".to_string(),
            protocol: Protocol::OpenAiCompat,
            base_url: format!("http://127.0.0.1:{port}/v1"),
            path: "/chat/completions".to_string(),
            auth_style: AuthStyle::Bearer,
            auth_header: "authorization".to_string(),
            extra_headers: vec![
                ("User-Agent".to_string(), "todolist-agent/1.0".to_string()),
                ("x-opencode-session".to_string(), "sess-123".to_string()),
            ],
            search_support: false,
            search_system_hint: None,
            key_required: true,
        };
        let provider = OpenAiCompatProvider::from_spec(&spec, store, "m".to_string()).unwrap();
        let out = provider
            .chat(
                &[ChatMessage::User("hi".to_string())],
                &[],
                CHAT_MAX_TOKENS,
                CancelToken::new(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert_eq!(out.text, "hi");
        let req = captured.lock().unwrap().clone();
        let lower = req.to_lowercase();
        assert!(
            lower.contains("authorization: bearer sk-go"),
            "Bearer 认证头应生效：{req}"
        );
        assert!(
            lower.contains("user-agent: todolist-agent/1.0"),
            "额外头 User-Agent 应生效：{req}"
        );
        assert!(
            lower.contains("x-opencode-session: sess-123"),
            "额外头 session 应生效：{req}"
        );
        assert!(req.starts_with("POST /v1/chat/completions"), "{req}");
    }

    #[tokio::test]
    async fn unsupported_search_fails_closed() {
        let store: Arc<dyn SecretStore> =
            Arc::new(crate::agent::secret::FakeSecretStore::with_slot("zhipu", "k"));
        let spec = ResolvedProvider {
            provider_id: "zhipu".to_string(),
            protocol: Protocol::AnthropicCompat,
            base_url: "https://example.invalid".to_string(),
            path: "/v1/messages".to_string(),
            auth_style: AuthStyle::Bearer,
            auth_header: "authorization".to_string(),
            extra_headers: Vec::new(),
            search_support: false,
            search_system_hint: None,
            key_required: true,
        };
        let provider = AnthropicCompatProvider::from_spec(&spec, store, "glm-5.3".into()).unwrap();
        let err = provider
            .search("q", 1, CancelToken::new(), Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(err.class(), CLASS_CONFIG, "未声明搜索支持 → 不构建请求");

        // OpenAI 协议搜索未实现 → fail closed
        let store2: Arc<dyn SecretStore> =
            Arc::new(crate::agent::secret::FakeSecretStore::with_slot("openai", "k"));
        let spec2 = ResolvedProvider {
            provider_id: "openai".to_string(),
            protocol: Protocol::OpenAiCompat,
            base_url: "https://example.invalid".to_string(),
            path: "/chat/completions".to_string(),
            auth_style: AuthStyle::Bearer,
            auth_header: "authorization".to_string(),
            extra_headers: Vec::new(),
            search_support: false,
            search_system_hint: None,
            key_required: true,
        };
        let p2 = OpenAiCompatProvider::from_spec(&spec2, store2, "gpt-5.6-sol".into()).unwrap();
        let err2 = p2
            .search("q", 1, CancelToken::new(), Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(err2.class(), CLASS_CONFIG);
    }

    #[tokio::test]
    async fn keyless_provider_sends_without_auth_header() {
        let body = r#"{"choices":[{"message":{"content":"hi"}}]}"#;
        let (port, captured) = spawn_capture(body);
        // 无密钥 + key_required=false → 不带认证头
        let store: Arc<dyn SecretStore> =
            Arc::new(crate::agent::secret::FakeSecretStore::new(None));
        let spec = ResolvedProvider {
            provider_id: "custom".to_string(),
            protocol: Protocol::OpenAiCompat,
            base_url: format!("http://127.0.0.1:{port}/v1"),
            path: "/chat/completions".to_string(),
            auth_style: AuthStyle::Bearer,
            auth_header: "authorization".to_string(),
            extra_headers: Vec::new(),
            search_support: false,
            search_system_hint: None,
            key_required: false,
        };
        let provider = OpenAiCompatProvider::from_spec(&spec, store, "local".to_string()).unwrap();
        let out = provider
            .chat(
                &[ChatMessage::User("hi".to_string())],
                &[],
                CHAT_MAX_TOKENS,
                CancelToken::new(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert_eq!(out.text, "hi");
        let req = captured.lock().unwrap().clone().to_lowercase();
        assert!(
            !req.contains("authorization:"),
            "keyless 不应带认证头：{req}"
        );
    }

    #[tokio::test]
    async fn key_required_missing_key_is_rejected() {
        let store: Arc<dyn SecretStore> =
            Arc::new(crate::agent::secret::FakeSecretStore::new(None));
        let spec = ResolvedProvider {
            provider_id: "deepseek".to_string(),
            protocol: Protocol::AnthropicCompat,
            base_url: "https://example.invalid".to_string(),
            path: "/v1/messages".to_string(),
            auth_style: AuthStyle::ApiKeyHeader,
            auth_header: "x-api-key".to_string(),
            extra_headers: Vec::new(),
            search_support: true,
            search_system_hint: None,
            key_required: true,
        };
        let provider = AnthropicCompatProvider::from_spec(&spec, store, "m".into()).unwrap();
        let err = provider
            .search("q", 1, CancelToken::new(), Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(err.class(), CLASS_MISSING_CREDENTIAL);
    }

    #[tokio::test]
    async fn http_get_json_reads_body_and_rejects_redirect() {
        let (port, _c) = spawn_mock(|s| {
            read_request(s);
            use std::io::Write;
            let body = r#"{"data":[{"id":"m1"}]}"#;
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.flush();
        });
        let body = http_get_json(
            &format!("http://127.0.0.1:{port}/v1/models"),
            &[],
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert!(body.contains("m1"));

        // 302 → 拒绝跟随（Redirect 分类）
        let (port2, _c2) = spawn_mock(|s| {
            read_request(s);
            use std::io::Write;
            let _ = s.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: /x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            let _ = s.flush();
        });
        let err = http_get_json(
            &format!("http://127.0.0.1:{port2}/v1/models"),
            &[],
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert_eq!(err.class(), CLASS_REDIRECT);
    }
}
