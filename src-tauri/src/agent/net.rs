//! 只读网页抓取：授权链接校验 + SSRF 防护 + 流式分页读取 + 轻量 HTML→文本。
//!
//! 仅允许抓取任务 `agent.links` 中**逐项精确命中**（trim 后相等）的链接；`file://` 不走网络，
//! 转本地路径后复用 [`sandbox::resolve_allowed`] 白名单。`http(s)` 每一跳（含重定向）都重新做
//! scheme + 解析结果 IP 校验，任一解析结果命中内网 / 保留段即整体拒绝；重定向手动跟随、至多
//! [`MAX_REDIRECTS`] 跳。单页不超过 [`PAGE_LIMIT`]，并按 [`ReadBudget`] 计入运行累计预算。
//!
//! 网页内容是**不可信数据**：本模块只做提取，不解释为指令。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{CONTENT_TYPE, LOCATION};
use reqwest::Url;

use super::sandbox::{self, ReadBudget};

/// 单次抓取一页上限：256KiB。
pub const PAGE_LIMIT: usize = 256 * 1024;
/// 单次抓取超时（同时不超过调用方剩余运行时限）。
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// 允许跟随的重定向跳数。
pub const MAX_REDIRECTS: usize = 3;

/// 抓取结果：文本 + 本次计入预算的原始字节数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub text: String,
    pub bytes: u64,
}

/// 是否应拒绝连接该 IP（SSRF / 内网 / 保留段）。
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_v4(mapped);
            }
            is_blocked_v6(v6)
        }
    }
}

fn is_blocked_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _d] = ip.octets();
    a == 0                                   // 0.0.0.0/8（含 unspecified）
        || a == 10                           // 10/8
        || a == 127                          // 127/8（loopback）
        || (a == 100 && (64..=127).contains(&b)) // 100.64/10（CGNAT）
        || (a == 169 && b == 254)            // 169.254/16（link-local）
        || (a == 172 && (16..=31).contains(&b)) // 172.16/12
        || (a == 192 && b == 168)            // 192.168/16
        || (a == 192 && b == 0 && c == 0)    // 192.0.0/24
        || (a == 198 && (b == 18 || b == 19)) // 198.18/15（benchmark）
        || a >= 240                          // 240/4（multicast + reserved + broadcast）
}

fn is_blocked_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    ip.is_unspecified()                       // ::
        || ip.is_loopback()                   // ::1
        || (seg[0] & 0xfe00) == 0xfc00        // fc00::/7（unique local）
        || (seg[0] & 0xffc0) == 0xfe80        // fe80::/10（link-local）
        || (seg[0] & 0xff00) == 0xff00        // ff00::/8（multicast）
        || (seg[0] == 0x2001 && seg[1] == 0x0db8) // 2001:db8::/32（文档保留）
}

/// 抓取授权链接的一页。
///
/// `authorized` 为任务 `agent.links`（已校验 / trim）；`url` trim 后必须与其中一项完全相等。
pub async fn fetch(
    url: &str,
    offset: u64,
    authorized: &[String],
    roots: &[PathBuf],
    budget: &mut ReadBudget,
    remaining: Duration,
) -> Result<Fetched, String> {
    let target = url.trim();
    if target.is_empty() {
        return Err("链接不能为空".to_string());
    }
    if !authorized.iter().any(|l| l.trim() == target) {
        return Err(format!("链接未获授权：{target}"));
    }
    let lower = target.to_ascii_lowercase();

    if lower.starts_with("file://") {
        let path = file_url_to_path(target)?;
        let mut guard = sandbox::resolve_allowed(Path::new(&path), roots)?;
        if guard.is_dir() {
            return Err("该链接指向目录，无法读取".to_string());
        }
        let bytes = guard.read_page(offset, budget)?;
        return Ok(Fetched {
            bytes: bytes.len() as u64,
            text: String::from_utf8_lossy(&bytes).to_string(),
        });
    }

    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return Err("仅支持 http:// / https:// / file:// 链接".to_string());
    }

    fetch_http(target, offset, budget, remaining).await
}

/// `file://` → 本地路径（仅做形状转换；白名单由调用方 `resolve_allowed` 负责）。
fn file_url_to_path(url: &str) -> Result<String, String> {
    let rest = &url["file://".len()..];
    let rest = rest.split('#').next().unwrap_or(rest);
    let rest = rest.split('?').next().unwrap_or(rest);
    let decoded = percent_decode(rest);
    let trimmed = decoded.trim_start_matches('/');
    if trimmed.is_empty() {
        return Err("file:// 链接缺少路径".to_string());
    }
    Ok(trimmed.replace('/', "\\"))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

async fn fetch_http(
    start: &str,
    offset: u64,
    budget: &mut ReadBudget,
    remaining: Duration,
) -> Result<Fetched, String> {
    let total = REQUEST_TIMEOUT.min(remaining);
    if total.is_zero() {
        return Err("运行剩余时限不足".to_string());
    }
    let deadline = tokio::time::Instant::now() + total;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "构建 HTTP 客户端失败".to_string())?;

    let mut current = Url::parse(start.trim()).map_err(|_| "链接格式非法".to_string())?;

    for hop in 0..=MAX_REDIRECTS {
        if !matches!(current.scheme(), "http" | "https") {
            return Err("仅支持 http:// / https:// 链接".to_string());
        }
        check_host(&current, deadline).await?;

        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return Err("抓取超时".to_string());
        }
        let resp = client
            .get(current.clone())
            .timeout(left)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    "抓取超时".to_string()
                } else {
                    format!("网络请求失败：{e}")
                }
            })?;

        let status = resp.status();
        if status.is_redirection() {
            if hop == MAX_REDIRECTS {
                return Err(format!("重定向次数过多（上限 {MAX_REDIRECTS}）"));
            }
            let location = resp
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "重定向缺少 Location".to_string())?;
            current = current
                .join(location)
                .map_err(|_| "重定向地址非法".to_string())?;
            continue;
        }
        if !status.is_success() {
            return Err(format!("HTTP 状态码 {}", status.as_u16()));
        }

        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        if !is_text_content_type(content_type.as_deref()) {
            return Err("非文本内容，已拒绝抓取".to_string());
        }

        let page = read_page_stream(resp, offset).await?;
        budget.reserve(page.len() as u64)?;
        let raw = String::from_utf8_lossy(&page).to_string();
        let text = if is_html_content_type(content_type.as_deref()) {
            html_to_text(&raw)
        } else {
            raw
        };
        return Ok(Fetched {
            text,
            bytes: page.len() as u64,
        });
    }
    Err("重定向次数过多".to_string())
}

/// 解析目标 host，任一结果命中拒绝段即整体拒绝（防 DNS 多 A 记录绕过）。
async fn check_host(url: &Url, deadline: tokio::time::Instant) -> Result<(), String> {
    let host = url.host_str().ok_or_else(|| "链接缺少主机名".to_string())?;
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return if is_blocked_ip(ip) {
            Err("目标地址不允许访问".to_string())
        } else {
            Ok(())
        };
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "链接缺少端口".to_string())?;
    let left = deadline.saturating_duration_since(tokio::time::Instant::now());
    if left.is_zero() {
        return Err("抓取超时".to_string());
    }
    let addrs = tokio::time::timeout(left, tokio::net::lookup_host((host, port)))
        .await
        .map_err(|_| "解析主机名超时".to_string())?
        .map_err(|_| "无法解析主机名".to_string())?;
    let mut any = false;
    for addr in addrs {
        any = true;
        if is_blocked_ip(addr.ip()) {
            return Err("目标地址不允许访问".to_string());
        }
    }
    if !any {
        return Err("无法解析主机名".to_string());
    }
    Ok(())
}

/// 流式读取一页：跳过 `offset` 字节后收集至多 [`PAGE_LIMIT`]（不整包缓冲）。
async fn read_page_stream(resp: reqwest::Response, offset: u64) -> Result<Vec<u8>, String> {
    let mut stream = resp.bytes_stream();
    let mut skipped: u64 = 0;
    let mut page: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("读取响应失败：{e}"))?;
        let mut slice = &chunk[..];
        if skipped < offset {
            let need = (offset - skipped) as usize;
            if slice.len() <= need {
                skipped += slice.len() as u64;
                continue;
            }
            slice = &slice[need..];
            skipped = offset;
        }
        if page.len() >= PAGE_LIMIT {
            break;
        }
        let room = PAGE_LIMIT - page.len();
        if slice.len() > room {
            page.extend_from_slice(&slice[..room]);
            break;
        }
        page.extend_from_slice(slice);
    }
    Ok(page)
}

/// 文本类响应放行；`image/*` / `application/octet-stream` 等拒绝。缺失 Content-Type 视为文本。
pub fn is_text_content_type(content_type: Option<&str>) -> bool {
    let Some(ct) = content_type else {
        return true;
    };
    let base = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if base.is_empty() {
        return true;
    }
    base.starts_with("text/")
        || base == "application/json"
        || base == "application/xml"
        || base == "application/xhtml+xml"
        || base.ends_with("+json")
        || base.ends_with("+xml")
}

fn is_html_content_type(content_type: Option<&str>) -> bool {
    content_type
        .map(|ct| {
            let base = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
            base == "text/html" || base == "application/xhtml+xml"
        })
        .unwrap_or(false)
}

fn tag_name(tag: &str) -> &str {
    let t = tag.trim_start().trim_start_matches('/');
    let end = t
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .unwrap_or(t.len());
    &t[..end]
}

/// HTML → 纯文本：去掉 `<script>` / `<style>` 内容与所有标签，解码基础实体，压缩空白。
pub fn html_to_text(html: &str) -> String {
    let mut stripped = String::with_capacity(html.len());
    let mut i = 0usize;
    while let Some(rel) = html[i..].find('<') {
        let lt = i + rel;
        stripped.push_str(&html[i..lt]);
        let Some(gt_rel) = html[lt..].find('>') else {
            break;
        };
        let gt = lt + gt_rel;
        let name = tag_name(&html[lt + 1..gt]).to_ascii_lowercase();
        if name == "script" || name == "style" {
            let close = format!("</{name}");
            if let Some(close_rel) = html[gt + 1..].to_ascii_lowercase().find(&close) {
                let close_at = gt + 1 + close_rel;
                if let Some(end_rel) = html[close_at..].find('>') {
                    stripped.push(' ');
                    i = close_at + end_rel + 1;
                    continue;
                }
            }
            i = html.len();
            break;
        }
        stripped.push(' ');
        i = gt + 1;
    }
    if i < html.len() {
        stripped.push_str(&html[i..]);
    }
    collapse_whitespace(&decode_entities(&stripped))
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi) = s[i..].find(';').filter(|p| *p <= 10) {
                let entity = &s[i + 1..i + semi];
                if let Some(ch) = decode_entity(entity) {
                    out.push(ch);
                    i += semi + 1;
                    continue;
                }
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity.to_ascii_lowercase().as_str() {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let num = entity.strip_prefix('#')?;
            let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                num.parse::<u32>().ok()?
            };
            char::from_u32(code)
        }
    }
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_ips_allowed() {
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700::1111"] {
            assert!(!is_blocked_ip(ip.parse().unwrap()), "应放行：{ip}");
        }
    }

    #[test]
    fn private_and_reserved_ips_blocked() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.20.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "100.64.0.1",
            "198.18.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_blocked_ip(ip.parse().unwrap()), "应拒绝：{ip}");
        }
    }

    #[test]
    fn html_becomes_plain_text() {
        let html = "<html><head><style>p{color:red}</style>\
                    <script>alert('x')</script></head>\
                    <body><p>Hello&nbsp; <b>World</b> &amp; more</p>\
                    <div>line&#50;</div></body></html>";
        assert_eq!(html_to_text(html), "Hello World & more line2");
    }

    #[test]
    fn text_content_type_filter() {
        assert!(is_text_content_type(Some("text/html; charset=utf-8")));
        assert!(is_text_content_type(Some("application/json")));
        assert!(is_text_content_type(Some("application/xml")));
        assert!(is_text_content_type(None));
        assert!(!is_text_content_type(Some("image/png")));
        assert!(!is_text_content_type(Some("application/octet-stream")));
    }

    #[tokio::test]
    async fn unauthorized_url_rejected_without_request() {
        let mut budget = ReadBudget::new();
        let err = fetch(
            "https://evil.test/x",
            0,
            &["https://ok.test/x".to_string()],
            &[],
            &mut budget,
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(err.contains("未获授权"), "应拒绝未授权链接：{err}");
        assert_eq!(budget.remaining(), sandbox::READ_BUDGET_LIMIT);
    }

    #[tokio::test]
    async fn ssrf_target_is_blocked() {
        let mut budget = ReadBudget::new();
        let err = fetch(
            "http://127.0.0.1/",
            0,
            &["http://127.0.0.1/".to_string()],
            &[],
            &mut budget,
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(err.contains("不允许访问"), "回环地址应被拒绝：{err}");
    }

    #[tokio::test]
    async fn file_url_inside_and_outside_roots() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = root.path().join("a.txt");
        std::fs::write(&file, b"local-content").unwrap();
        let link = format!(
            "file:///{}",
            file.to_string_lossy().replace('\\', "/")
        );

        let mut budget = ReadBudget::new();
        let got = fetch(
            &link,
            0,
            std::slice::from_ref(&link),
            &[root.path().to_path_buf()],
            &mut budget,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(got.text, "local-content");
        assert_eq!(got.bytes, 13);
        assert_eq!(budget.remaining(), sandbox::READ_BUDGET_LIMIT - 13);

        // 授权根外 → 拒绝
        let mut budget = ReadBudget::new();
        let err = fetch(
            &link,
            0,
            std::slice::from_ref(&link),
            &[other.path().to_path_buf()],
            &mut budget,
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(err.contains("授权根"), "根外文件应拒绝：{err}");
    }
}
