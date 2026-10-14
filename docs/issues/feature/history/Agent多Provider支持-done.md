# Agent 多 Provider 支持

> Status: done（用户验收通过 2026-10-14）｜ 责任: agent ｜ 创建: 2026-10-14 ｜ 关联: ADR-0009（修订 ADR-0008 / ADR-0005）；PDR `docs/pdr/10-app`（01-spec §9 / 02-interfaces §6 / 03-nfr §6 / 04-current）

背景：Agent 配置当前仅支持 DeepSeek（`provider.rs` 固定端点 `https://api.deepseek.com/anthropic`，Anthropic Messages 协议；单槽 DPAPI 密钥；`settings.yaml` 无 provider 字段）。本次升级：双协议适配（Anthropic Messages 兼容 + OpenAI 兼容）+ 9 个内置预设 + 自定义端点（协议可选、key 可空）；模型列表按预设动态拉取；搜索能力按预设声明降级并在 UI 标注；凭据分槽 + 旧配置零手工迁移；修订 ADR-0005/0008。

预设与协议（2026-10-14 官方文档查证）：

| 预设 | 协议 | base URL | 搜索 | /models |
|---|---|---|---|---|
| OpenAI | OpenAI 兼容 | `https://api.openai.com/v1` | 无（降级） | ✅ |
| DeepSeek | Anthropic 兼容（现状沿用） | `https://api.deepseek.com/anthropic` | 现状依赖、文档未确认（未实测 → 待验证） | 仅 OpenAI 侧有 |
| Anthropic | Anthropic 原生 | `https://api.anthropic.com/v1` | ✅ `web_search_20250305`（仍有效） | ✅ |
| OpenCode Go | 混合（按模型） | `https://opencode.ai/inference/go/v1` | 未知（降级） | ✅ |
| OpenRouter | OpenAI 兼容 | `https://openrouter.ai/api/v1` | 无（`:online` 为后续） | ✅ |
| Gemini | OpenAI 兼容 | `https://generativelanguage.googleapis.com/v1beta/openai/` | 无（grounding 为后续） | ✅ |
| Qwen | Anthropic 兼容 | `dashscope.aliyuncs.com/apps/anthropic` / `maas.qwencloudapi.com/apps/anthropic`（域名待定稿） | ✅（需 billing header 约定，实现时验证） | 无（回退输入） |
| Zhipu | Anthropic 兼容 | `https://open.bigmodel.cn/api/anthropic` | 待验证 | 无（回退输入） |
| Kimi | OpenAI 兼容 | `https://api.moonshot.ai/v1` | 无（builtin `$web_search` 为后续） | ✅ |

备注：OpenCode Go 需预设内置头（User-Agent、`x-opencode-session`），认证为 Bearer；混合协议按模型映射处理（chat 模型走 OpenAI 适配器、messages 模型走 Anthropic 适配器；responses 模型 v1 不支持、列表标注；未知模型默认 OpenAI 兼容——官方端点表未列的 6 个模型〔deepseek-flash、glm-5.1、omen-alpha、minimax-m2.5、qwen3.6-plus、qwen3.7-max〕按未知处理）。认证头按预设配置：Anthropic 原生 / DeepSeek 用 `x-api-key`，Qwen / Zhipu / Kimi 等兼容端点用 Bearer。

预设模型建议（查证 2026-10-14；默认模型细节阶段定稿）：OpenAI `gpt-5.6-sol` / `gpt-6-astra`；Anthropic `claude-sonnet-5` / `claude-opus-5`；DeepSeek `deepseek-flash` / `deepseek-v4-pro`；OpenCode Go `deepseek-v4.1-flash`；OpenRouter `anthropic/claude-sonnet-5` / `openai/gpt-5.6-sol`；Gemini `gemini-3.5-flash` / `gemini-3.1-pro-preview`；Qwen `qwen3.8-max` / `qwen3.8-flash`；Zhipu `glm-5.3` / `glm-5.3-flash`；Kimi `kimi-k3` / `kimi-k2.6`。

Qwen 搜索约定（官方）：`x-anthropic-billing-header: cc_entrypoint=cli;` 须作为 **system 内容块**传入（非 HTTP header），否则 web_search 不生效；端点域名两口径（`dashscope.aliyuncs.com/apps/anthropic` 与 `maas.qwencloudapi.com/apps/anthropic`），实现时定稿。

OpenCode Go 协议映射（官方文档 2026-10-14；part 2 按此收敛，替换近似 needle 规则）：chat/completions＝`glm-5.3-flash`、`glm-5.3`、`glm-5.2`、`kimi-k3`、`kimi-k2.7-code`、`kimi-k2.6`、`longcat-2.0`、`deepseek-v4.1-flash`、`deepseek-v4-pro`、`deepseek-v4-flash`、`deepseek-v4-flash-vision-exp`、`mimo-v2.6-flash`、`mimo-v2.6-pro`、`mimo-v2.5`、`mimo-v2.5-pro`、`hy4-preview`、`hy3`、`space-bunny`、`longcat-2.5-preview-free`、`step-5-preview-free`、`cat-jam-free`；messages＝`claude-haiku-5-5`、`minimax-m3`、`minimax-m2.7`、`qwen3.8-max`、`qwen3.8-flash`、`qwen3.7-plus`；responses（v1 不支持、构造报错）＝`grok-4.7`、`grok-4.6`、`gpt-6-luna`、`gpt-5.6-luna`、`muse-spark-1.3-contributor`、`muse-spark-1.2-contributor`；官方未列（按未知→OpenAI 默认）＝`deepseek-flash`、`glm-5.1`、`omen-alpha`、`minimax-m2.5`、`qwen3.6-plus`、`qwen3.7-max`。

## 完成判据

- 现有 deepseek 配置零手工迁移（旧 key 自动归入 deepseek 槽；`agent.provider` 缺失默认 deepseek）。
- 9 预设一键切换可对话；自定义端点（协议可选、key 可空）可接入并对话。
- 模型列表按预设动态拉取（支持处），失败 / 不支持回退自由输入；记住每个 provider 上次模型。
- 搜索仅对声明支持的预设启用（独立请求，不与 fs 工具混用）；不支持者 UI 标注；不引入第三方搜索服务。
- probe 改为对当前 provider/model 的最小 chat 请求（保留二次确认）；执行开关允许无 key 自定义端点。
- ADR-0005/0008 修订；PDR（01-spec / 02-interfaces / 04-current）同步；配置区 UI 由 designer 重构。
- 验证：Rust 单测（双适配器 fixture / 迁移 / 存储）+ FakeProvider + clippy 干净 + `pnpm build` + 前端断言；真实联网不纳入自动验证。

## 步骤

- [x] 规划（grill 定案 2026-10-14；预设事实查证完成）
- [x] 骨架（最小可运行切片 + 证据：预设注册表 + 参数化 Anthropic 适配器（deepseek / anthropic 两预设）+ 凭据分槽与迁移 + 最小 UI 接线；证据已展示，待用户 review）
- [x] 细节（OpenAI 兼容适配器 + 9 预设 + 动态模型列表 + probe 改造 + UI 重构（designer））
- [x] 异常（拉取失败回退、校验、错误映射、无 key 端点、非 https 提示等；收口修复批次 9 项 + `storage.rs` BOM 残留）
- [x] 回顾（ADR-0009 / PDR / README / CONTEXT 已同步；并入 1.7.0、暂不打 tag；产物重打）

## 结果

- 交付：9 内置预设（openai / deepseek / anthropic / opencode-go / openrouter / gemini / qwen / zhipu / kimi）+ 自定义端点（协议可选、key 可空）、双协议适配、动态模型列表、搜索按 provider 降级、凭据分槽 + 旧单槽迁移、配置弹窗重构、probe 最小 chat。
- 验证：`cargo test --all-targets` **234/234**（feature **233/233**）、clippy 干净、`pnpm build`、前端断言全绿（15 / 16 / 14 / 13 / 6）、隔离 CDP **83 PASS / 0 FAIL / 0 LIMITED**。
- 文档：ADR-0009（修订 ADR-0005/0008）、PDR（01-spec §9 / 02-interfaces §6 / 03-nfr §6 / 04-current）、README、CONTEXT 已同步。
- 状态：**用户验收通过（2026-10-14）**；并入 1.7.0（已打 tag `v1.7.0`）；产物按最新代码重打（见 Log）。

## Log

- 2026-10-14 创建。grill 定案：双协议、9 预设 + 自定义、动态模型列表、搜索降级、ADR 修订、凭据分槽迁移、UI 交 designer。
- 2026-10-14 预设事实查证（官方文档）：Qwen anthropic 端点支持 web_search；Anthropic `web_search_20250305` 仍有效（另有 20260209/20260318 可选升级）；DeepSeek anthropic 端点未查到 web_search 文档（现状依赖，待验证）；OpenCode Go = 混合协议网关（chat/messages/responses 按模型，需内置头）。待定：OpenCode Go 混合协议处理方式（按模型映射 vs 仅 OpenAI 子集）。
- 2026-10-14 用户确认：OpenCode Go 混合协议按模型映射处理（chat 模型走 OpenAI 适配器、messages 模型走 Anthropic 适配器；responses 模型 v1 不支持、列表标注；未知模型默认 OpenAI 兼容）。
- 2026-10-14 ADR-0009 落地（多 Provider 边界修订）：0005 / 0008 加修订标注、README 登记、03-nfr 例外条款同步；引用 sweep 完成（两处代码注释随骨架 / 细节更新：`provider.rs` 端点注释、`agent.ts` 契约注释）。
- 2026-10-14 预设模型与端点细节查证（官方文档）：各预设建议模型清单已录（见上）；OpenCode Go 39 个模型中 6 个协议归属官方未列（按未知处理）；Qwen 搜索需 system 内容块 `x-anthropic-billing-header: cc_entrypoint=cli;`；Qwen 端点域名两口径待实现定稿。
- 2026-10-14 骨架完成（fix-1）：`presets.rs` 注册表（deepseek / anthropic；`agent_list_providers` 单一事实源）+ `AnthropicCompatProvider` 参数化（deepseek 端点 URL 逐字不变，`base_url` + `messages_path` 拆分）+ `agent.provider`（缺失默认 deepseek、未知 fail closed）+ 凭据分槽与旧单槽自动迁移（幂等、原子、坏文件不覆盖）+ 最小 UI 接线。验证：`cargo test --all-targets` **202/202**（含 `agent-test-provider`；+11 新测试）、clippy 干净、`pnpm build`、`git diff --check` 通过；迁移为测试级运行证据。GUI 冒烟未做（无头环境），待用户本地 `pnpm tauri dev` 复核。顺手修一处既有 clippy warning（`reminder.rs:654`，行为不变，可回退）。
- 2026-10-14 用户复核：当前仅 deepseek / anthropic 两预设为预期内（骨架范围）；细节阶段启动——车道 A（fix-1 续用：OpenAI 兼容适配器 + runner 工具调用抽象 + 9 预设全量数据 + 协议解析）、车道 B（designer：配置弹窗配置区重构，按冻结契约接线）。默认模型定稿：openai=`gpt-5.6-sol`、anthropic=`claude-sonnet-5`、opencode-go=`deepseek-v4.1-flash`、openrouter=`anthropic/claude-sonnet-5`、gemini=`gemini-3.5-flash`、qwen=`qwen3.8-max`、zhipu=`glm-5.3`、kimi=`kimi-k3`（deepseek 维持 `deepseek-flash`）。细节接口冻结（供前端并行）：`agent_list_providers` 增 `protocol`/`is_custom`；新增 `agent_list_models`；settings 增 `agent.custom:{protocol,base_url}` 与 `agent.model_by_provider`。后端 part 2（custom / 模型列表 / probe / keyless）随后续用同一会话。
- 2026-10-14 细节车道 B（designer）完成：配置区重构（Provider 选择 9+自定义、可编辑模型组合框〔逐项协议 / 不可用标注、刷新、内存缓存不落盘〕、自定义端点区〔协议 / baseURL / 可空 key / 非 https 轻提示〕、搜索降级标注、keyless 门禁、per-provider 模型记忆、文案）。`pnpm build` / `tsc --noEmit` / `git diff --check` 通过；静态渲染证据三张（preset / custom / narrow360，`%TEMP%\opencode\agent-preview\`）；未真机冒烟（无 Tauri 运行时），交集成验证。
- 2026-10-14 协议值口径统一为 `anthropic` / `openai`（后端 `Protocol::as_str` 口径；前端 `AgentProtocol` / `PROTOCOL_OPTIONS` 已对齐，`normalizeProtocol` 容错旧式写法）。
- 2026-10-14 细节车道 A part 1（fix-1）完成：9 预设全量（含 opencode-go 混合协议、Qwen system 约定、Bearer / x-api-key）、OpenAI 兼容适配器（chat/completions 请求 / 响应、tool_calls）、runner 协议无关工具调用抽象、共享 transport 不变量、`ResolvedProvider` 统一构造。验证：`cargo test --all-targets` **215/215**（+13）、clippy 干净、`pnpm build`、`git diff --check` 通过。遗留（part 2）：config `custom` / `model_by_provider`、`agent_list_models`、probe 改造（最小 chat）、keyless custom、native_search 按 search_support 门控、opencode-go 映射按官方表收敛（当前为近似 needle 规则）。
- 2026-10-14 细节车道 A part 2（fix-1）完成：opencode-go 官方全表收敛（精确匹配 + 漂移注释）、config `custom` / `model_by_provider`（保存校验、仅 custom 时生效）、`resolve_custom`（含 `/v1` 路径规则）、keyless custom（无认证头、不报 MissingCredential）、`agent_list_models`（短 deadline、失败 → source none、逐模型协议标注）、probe 改最小 chat（保留二次确认）、native_search 按 search_support 门控。验证：`cargo test --all-targets` **227/227**（+12）、clippy 干净、`pnpm build`、`git diff --check` 通过。
- 2026-10-14 进入集成验证：验证车道（独立 fixer 会话：全量批次 + 前端断言脚本 + 隔离 CDP 集成尝试）与 @oracle 独立审阅（安全不变量 / 迁移 / keyless / 协议等价 / ADR-0009 一致性）并行；随后按结果修复（如有）→ PDR 文档同步 → 用户真机复核。
- 2026-10-14 @oracle 独立审阅完成（收口前 gate）：**无 HIGH**；核心不变量独立确认通过（密钥不回 JS / 迁移幂等原子 / 禁重定向与上限 / 错误分类 / 搜索独立 / fail-closed / keyless 不绕 consent 与付费门禁 / 双协议等价 / ADR-0009 一致性；另独立复跑 `cargo test` 227/227）。**待修清单**：中-1 `agent_list_models` 对 custom 会把已存密钥发往调用方任意 URL（修复：仅当入参与已保存 `agent.custom` 一致时才附带密钥）；中-2 自定义 anthropic 协议固定 Bearer（修复：改用 `x-api-key`，与预设一致）；低-1 deepseek `models_url` 属 OpenAI 侧应改 Bearer；低-2 `read_map` 成功路径未 zeroize 解密明文；低-3 scheme 校验后端大小写敏感（改忽略大小写）；低-4 Anthropic 空 `tools: []` 应省略（与 OpenAI 对齐）；低-5 opencode-go 模型列表缺 `x-opencode-session` 头（经 resolve_provider 生成）。信息级（记录不修）：保存不预校验模型可用性（运行期 fail-closed）、`present()` 读触发迁移写盘（幂等）、`drain_once` 不复查 `execution_enabled`（既有行为）。修复在集成验证（fix-2）完成后由 fix-1 会话执行，随后定向复验。
- 2026-10-14 集成验证（fix-2，独立会话）完成：全量批次 227/227、clippy 干净、`pnpm build`、`git diff --check` 通过；前端断言 appBar 15 / completedGroups 16 / taskMove 14 / agentTask 13 + **新增 `src/agentConfig.verify.cjs` 6** 全绿；隔离 CDP 集成（假 provider、零真实网络、identifier 隔离、真实配置逐字节备份还原且 hash 校验通过）：**UI 会话 32 PASS / 0 FAIL / 0 LIMITED；迁移会话 5 PASS / 0 FAIL / 1 LIMITED**（覆盖：9+自定义下拉、切换保存持久化、自定义端点与非 https 提示、keyless 门禁、模型列表回退、分槽隔离、模型记忆、回归；旧单槽 → JSON map 迁移经 IPC + 磁盘核对）。发现 2 项：①（验证基建）`agent-test-provider` 未覆盖 `agent_list_models`，假构建仍会真实联网 → 加 feature 门禁；②（健壮性）settings.yaml 带 UTF-8 BOM 时 agent 配置静默回退且保存报错误导 → 读取剥离 BOM。诚实披露：验证首轮因种子 BOM 曾意外触发一次对 `api.deepseek.com/models` 的请求尝试（无任务数据、结果 none）。
- 2026-10-14 收口修复批次启动（fix-1 会话）：中-1 密钥边界（仅与已保存 custom 一致时带密钥）/ 中-2 自定义 anthropic 改 `x-api-key` / 低-1 deepseek 模型列表认证 / 低-2 `read_map` 成功路径 zeroize / 低-3 scheme 大小写 / 低-4 空 `tools` 省略 / 低-5 opencode-go 列表头 / 验证基建 feature 门禁 / BOM 剥离；随后定向复验（fix-2 会话，含迁移 UI LIMITED 用例离线复跑）。
- 2026-10-14 收口修复批次（fix-1）完成（9/9）：密钥边界（归一化比较 + 不匹配不带认证头）、自定义 anthropic 改 `x-api-key`、deepseek 列表认证 Bearer（新增 `models_auth`）、`read_map` 成功路径 zeroize、scheme 大小写不敏感、Anthropic 空 `tools` 省略、opencode-go 列表头单一来源（`preset_extra_headers`）、`agent_list_models` feature 门禁（离线假列表，含 `supported:false` 项）、settings BOM 剥离。验证：`cargo test --all-targets` **233/233**（feature 构建 232/232，计数差异为测试门控设计）、clippy 干净、`pnpm build`、`git diff --check` 通过。
- 2026-10-14 定向复验启动（fix-2 会话）：全量批次核对 + 迁移会话离线复跑（目标 0 LIMITED）+ UI 会话回归（含 remote 假列表渲染 / 不可用标注新覆盖）。
- 2026-10-14 定向复验（fix-2）通过：全量批次与自报一致（`cargo test` 233/233、feature 232/232、clippy 干净、`pnpm build`、`git diff --check`）；前端断言全绿；CDP 合计 **83 PASS / 0 FAIL / 0 LIMITED**（迁移 16〔原 LIMITED 已离线覆盖〕+ UI remote 36〔覆盖 remote 渲染 / 不可用项标注〕+ UI none 31〔回退自由输入〕）；真实配置逐字节还原（hash 校验通过）。新发现（low）：`storage.rs` 设置解析未剥离 BOM（主题 / 功能栏等回退默认；非本特性路径，外部编辑器触发）——纳入收尾修复。诚实披露：复验首轮因种子脚本问题误用真实配置跑了一轮（feature 构建、离线、无真实网络；已逐字节还原）。
- 2026-10-14 收尾批次启动（fix-1）：① `storage.rs` BOM 剥离（复用 `strip_bom`）+ 测试；② PDR（01-spec §9 / 02-interfaces §6 / 03-nfr §6 / 04-current）与 README / CONTEXT 文档同步（按实现与验证数据；诚实口径；不 commit）。随后：用户真机复核。
- 2026-10-14 收尾批次（fix-1）完成：① `storage.rs` BOM 剥离（共享 `strip_bom` helper + storage 级测试）——`cargo test --all-targets` **234/234**（feature **233/233**）、clippy 干净、`pnpm build`、`git diff --check` 通过；② 文档同步完成：01-spec §9 / 02-interfaces §6 / 03-nfr §6 / 04-current / README / CONTEXT（新增术语 `Provider`），状态统一为「已实现；隔离集成复验通过（假 provider、零真实网络）；真实联网未实测；待用户验收」；CDP 83 数据已入 04-current。**待用户真机复核**；随后归档与版本策略。
- 2026-10-14 **用户验收通过**；文档状态同步（「待用户验收」→「用户验收通过（2026-10-14）」）；条目归档 `feature/history/Agent多Provider支持-done.md`；并入 1.7.0（未打 tag）；产物按最新代码重打（`scripts/build-release.ps1 -Version 1.7.0`）。
- 2026-10-14 产物重打完成：`release/TodoList_1.7.0_x64-setup.exe`（3,125,460 B）+ `TodoList_1.7.0_x64.exe`（8,798,720 B）+ 两份 `.sha256`；提交 / 打 tag / 推送与 Release 上传由用户执行。
- 2026-10-14 用户确认：**暂无其他供应商 key，不做真实联网测试**（记录于此）；文档口径保持「真实联网未实测」。1.7.0 **收敛**：提交并打 tag `v1.7.0`；推送 / GitHub Release 上传由用户执行。
