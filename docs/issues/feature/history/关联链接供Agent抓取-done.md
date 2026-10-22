# 关联链接供 Agent 抓取

> Status: done ｜ 责任: agent ｜ 创建: 2026-10-22 ｜ 关联: 关联材料独立板块与打开、Agent协助任务

## 背景与目标

上一步把链接 `agent.links` 只当作人工点击打开的快捷方式，不传给 Agent。现改为**链接也提供给 Agent**：新增只读抓取工具 `url_read`，并在初始 prompt 中列出授权材料（本地路径 + 链接），使模型知道有哪些材料可读、分别用什么工具读。

## 完成判据

- 新增客户端工具 `url_read({ url, offset? })`：仅 `http(s)`，GET，单次一页 ≤256 KiB，超时 20s（且不超过运行剩余时限），重定向最多 3 跳，HTML 去标签转文本，非文本（二进制）类型报错不读；
- **只允许抓取任务授权的链接**：`url` 必须精确命中 `agent.links`（trim 比较），未授权 → 工具报错；
- **SSRF 防护**：每一跳都拒绝回环、私网（10/8、172.16/12、192.168/16）、链路本地（169.254/16、fe80::/10）、唯一本地（fc00::/7）、`::1`、未指定地址、多播；多 IP 解析全量校验；重定向手动跟随以对每跳复检；
- `file://` 链接**转成路径后走 `fs_read` 同款白名单**（不在授权 roots 下即拒绝），不绕过沙箱；
- 抓取字节计入单次运行 5 MiB 读取预算与 `stats.bytes_read`；
- `RunSnapshot` 记录授权 `links`；`RunRecord` 记录本次实际抓取的 URL（去重）；
- 初始 prompt 列出授权材料：`路径：…` / `链接：…`，并声明 `url_read`；无材料时不出现空段；
- 既有行为不回退：`fs_list` / `fs_read` / `native_search`、预算、遗漏清单、搜索来源；
- `cargo test` 通过；接口文档同步工具与记录字段。

## 步骤

- [x] 规划（传递方式 / prompt 补全 / 工具规格 / SSRF / file:// 处理 / 预算与留档 / 文案已对齐；name 需求已由用户撤回）
- [x] 骨架（`url_read` 工具 + 授权校验 + prompt 材料清单）
- [x] 细节（SSRF 逐跳校验、重定向、HTML 转文本、分页与预算）
- [x] 异常（未授权 URL、私网地址、二进制响应、超时、超预算、file:// 越权）
- [x] 回顾（文档同步、条目归档）

## Log

- 2026-10-22 用户改判：链接也要给 Agent。对齐结论：新增 `url_read`；初始 prompt 补授权材料清单；`file://` 走 `fs_read` 白名单；抓取计入预算并留档；SSRF 逐跳校验。另：用户提出并随即撤销「材料 name 字段」需求，本次不做。
- 2026-10-22 实施：新增 `src-tauri/src/agent/net.rs`（授权链接精确匹配、`file://`→本地白名单、手动重定向 ≤3 跳、每跳 scheme+IP 校验、DNS 全量校验、20s 且受运行剩余时限约束、`bytes_stream()` 256KiB 截断、文本类型过滤、轻量 HTML→文本）；runner 增 `url_read` 工具、初始 prompt「可用材料」段、运行时 links 再校验、`web_reads` 去重记录；`RunSnapshot.links` / `RunRecord.web_reads`（旧记录兼容）；`reqwest` 加 `stream` feature。前端两处文案改为「路径与链接都只读提供给 Agent」。
- 2026-10-22 验证：`cargo test` 252 passed / 0 failed；`pnpm build` 通过。待用户真机冒烟（授权链接可抓、未授权被拒、内网链接被拒、file:// 越权被拒）后归档。
- 2026-10-22 用户验收通过。另：启动白屏经查为 dev 首次依赖优化（约 1s），非代码缺陷；已在 `index.html` 加首帧系统色兜底。归档。
