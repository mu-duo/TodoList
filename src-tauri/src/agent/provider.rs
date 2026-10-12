//! Provider：主 chat 请求与**独立** native web_search 请求（禁混用、禁跟随重定向）。
//!
//! 请求构造与响应解析均为纯函数（离线可测）；HTTP 仅经 [`DeepSeekProvider`] 走 Rust。
//! 错误分类用于 `error_class`，**绝不**包含密钥或原始响应体。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use super::secret::SecretStore;

/// Anthropic 兼容协议版本。
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// 服务端搜索工具类型（固定版本名）。
pub const SEARCH_TOOL_TYPE: &str = "web_search_20250305";
/// 服务端搜索工具名。
pub const SEARCH_TOOL_NAME: &str = "web_search";
/// 生产固定端点（**不可**经配置 / 环境任意改动）。
pub const DEFAULT_BASE_URL: &str = "https://api.deepseek.com/anthropic/v1";
/// chat 默认 max_tokens。
#[allow(dead_code)] // P2 runner：主 chat 循环消费。
pub const CHAT_MAX_TOKENS: u32 = 4096;
/// 辅助 search 默认 max_tokens（小）。
pub const SEARCH_MAX_TOKENS: u32 = 1024;
/// 响应体字节上限（防止内存放大 / 日志泄漏）。
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// 客户端整体超时（兜底；单请求另设 `deadline`）。
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

/// 构造主 chat 请求（客户端工具由调用方提供；**不**含服务端 web_search）。
#[allow(dead_code)] // P2 runner：主 chat 循环消费。
pub fn build_chat_request(model: &str, messages: Value, tools: Value, max_tokens: u32) -> Value {
    json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages,
        "tools": tools,
    })
}

/// 构造**独立** native search 请求：唯一工具为服务端 `web_search_20250305`。
pub fn build_search_request(model: &str, query: &str, max_uses: u32, max_tokens: u32) -> Value {
    json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": query }] }],
        "tools": [{ "type": SEARCH_TOOL_TYPE, "name": SEARCH_TOOL_NAME, "max_uses": max_uses }],
    })
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatOutcome {
    pub text: String,
    pub had_server_tool_use: bool,
    pub had_search_result: bool,
    pub citations: Vec<String>,
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
            _ => {}
        }
    }
    Ok(ChatOutcome {
        text,
        had_server_tool_use,
        had_search_result,
        citations,
    })
}

/// 网络 provider 抽象（Phase 2 runner 复用）。
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    #[allow(dead_code)] // P2 runner：主 chat 循环调用。
    async fn chat(
        &self,
        messages: Value,
        tools: Value,
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

/// DeepSeek（Anthropic 兼容端点）provider；HTTP 全在 Rust，密钥仅本机读取。
pub struct DeepSeekProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
    secret: Arc<dyn SecretStore>,
}

impl DeepSeekProvider {
    /// 生产构造：端点固定为官方域，禁跟随重定向；client 构建失败向上传播（fail closed）。
    pub fn new(secret: Arc<dyn SecretStore>, model: String) -> Result<Self, ProviderError> {
        Self::build(secret, model, DEFAULT_BASE_URL.to_string())
    }

    /// 仅测试 / `agent-test-provider` 特性可指定 base（如本地离线 fixture）。
    #[cfg(any(test, feature = "agent-test-provider"))]
    pub fn with_base_url(
        secret: Arc<dyn SecretStore>,
        model: String,
        base_url: String,
    ) -> Result<Self, ProviderError> {
        Self::build(secret, model, base_url)
    }

    fn build(
        secret: Arc<dyn SecretStore>,
        model: String,
        base_url: String,
    ) -> Result<Self, ProviderError> {
        // fail closed：构建失败必须向上传播，绝不回退到默认（会跟随重定向、无超时）client。
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| ProviderError::new(CLASS_TRANSPORT, "构建 HTTP 客户端失败"))?;
        Ok(Self {
            client,
            base_url,
            model,
            secret,
        })
    }

    async fn post(
        &self,
        body: Value,
        cancel: CancelToken,
        deadline: Duration,
    ) -> Result<(u16, String), ProviderError> {
        if cancel.is_cancelled() {
            return Err(ProviderError::new(CLASS_ABORTED, "已取消"));
        }
        let key = self
            .secret
            .get()
            .map_err(|_| ProviderError::new(CLASS_MISSING_CREDENTIAL, "读取本机密钥失败"))?
            .ok_or_else(|| ProviderError::new(CLASS_MISSING_CREDENTIAL, "未配置本机密钥"))?;

        let url = format!("{}/messages", self.base_url.trim_end_matches('/'));
        // 绝对 deadline：send 与 body 读取共享同一总时限（不随分块重置）。
        let deadline_at = tokio::time::Instant::now() + deadline;
        let request = self
            .client
            .post(&url)
            .header("x-api-key", key.as_str())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .timeout(deadline)
            .json(&body);

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
}

/// reqwest 错误 → ProviderError；超时一律归 `Aborted`（deadline），其余 `Transport`。
fn map_transport(err: reqwest::Error, context: &'static str) -> ProviderError {
    if err.is_timeout() {
        ProviderError::new(CLASS_ABORTED, "已超时")
    } else {
        ProviderError::new(CLASS_TRANSPORT, context)
    }
}

#[async_trait::async_trait]
impl Provider for DeepSeekProvider {
    async fn chat(
        &self,
        messages: Value,
        tools: Value,
        max_tokens: u32,
        cancel: CancelToken,
        deadline: Duration,
    ) -> Result<ChatOutcome, ProviderError> {
        let body = build_chat_request(&self.model, messages, tools, max_tokens);
        let (status, text) = self.post(body, cancel, deadline).await?;
        parse_chat_response(status, &text)
    }

    async fn search(
        &self,
        query: &str,
        max_uses: u32,
        cancel: CancelToken,
        deadline: Duration,
    ) -> Result<SearchOutcome, ProviderError> {
        let body = build_search_request(&self.model, query, max_uses, SEARCH_MAX_TOKENS);
        let (status, text) = self.post(body, cancel, deadline).await?;
        parse_search_response(status, &text)
    }
}

/// 测试用假 provider（仅替换网络，不绕过任何安全规则）；`#[cfg]` 门禁。
#[cfg(any(test, feature = "agent-test-provider"))]
#[allow(dead_code)] // 假实现字段供 P2 测试扩展。
pub struct FakeProvider {
    pub search: std::sync::Mutex<Result<SearchOutcome, ProviderError>>,
    pub chat: std::sync::Mutex<Result<ChatOutcome, ProviderError>>,
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
            })),
            delay: Duration::ZERO,
        }
    }
}

#[cfg(any(test, feature = "agent-test-provider"))]
#[async_trait::async_trait]
impl Provider for FakeProvider {
    async fn chat(
        &self,
        _messages: Value,
        _tools: Value,
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
        self.search.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let body = build_search_request("deepseek-flash", "DeepSeek API docs", 5, SEARCH_MAX_TOKENS);
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

    fn provider_at(port: u16) -> DeepSeekProvider {
        let store: Arc<dyn SecretStore> =
            Arc::new(crate::agent::secret::FakeSecretStore::new(Some("test-key")));
        DeepSeekProvider::with_base_url(
            store,
            "deepseek-flash".to_string(),
            format!("http://127.0.0.1:{port}/anthropic/v1"),
        )
        .unwrap()
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
}
