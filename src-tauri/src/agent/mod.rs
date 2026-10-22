//! Agent 协助 —— Phase 1 安全骨架。
//!
//! 覆盖：本机密钥（DPAPI）、非敏感配置（`settings.yaml` 的 `agent`）、任务意图
//! （任务 YAML `agent` 字段）、只读沙箱路径护栏、provider（chat / 独立 native search）、
//! 显式收费 probe 与原生 dialog 桥。
//!
//! 硬边界：密钥**绝不**回传 JS / 写日志 / 进错误串；HTTP 全在 Rust；所有 `*_in(base)`
//! 可在临时目录注入；Phase 1 **不做任何真实付费调用**，probe 仅在用户显式确认时才会
//! 走网络。Phase 2 的 runner / 调度 / 结果不在此实现。

pub mod clock;
pub mod commands;
pub mod config;
pub mod intent;
pub mod net;
pub mod presets;
pub mod provider;
pub mod record;
pub mod runner;
pub mod sandbox;
pub mod schedule;
pub mod secret;

pub use intent::TaskAgent;

/// 默认模型（不自动回退到其它模型）。
pub const DEFAULT_MODEL: &str = "deepseek-flash";
/// 默认单请求内服务端搜索次数上限（**非**运行总帽）。
pub const DEFAULT_MAX_USES: u32 = 5;
