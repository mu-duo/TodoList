# 提醒改造：默认三档 + 不再提醒

> Status: done ｜ 责任: agent ｜ 创建: 2026-10-13 ｜ 关联: PDR `docs/pdr/10-app/01-spec.md`（FR-APP-07/08）、`02-interfaces.md`、`04-current.md`；前序归档「任务详情元数据与截止提醒-done」

1.5.0 的提醒为「到期 + 可选提前 1 天」（任务 YAML `reminder_offset_days: 1`，opt-in）。用户提出改造：默认提醒更积极（T-7d / T-1d / T-1h + 到期，共最多 4 条，均默认开启）；关闭更简单（通知上「不再提醒」按钮 + 详情单一勾选框）；详情说明文案整段删除（用户不应当关心）。目标版本 1.6.0；1.5.0 已定稿不再改动。

### 已对齐共识（2026-10-13 grilling）
1. 版本：1.6.0；1.5.0 原样收尾（推送/发布在用户侧）。
2. 触发：带 `due` 的 open 任务默认在 T-7d/T-1d/T-1h 与到期时刻各提醒一次；date-only 以当天 09:00 为基准；7/1 天档=本地日历日回退、1 小时档=本地时刻减 1 小时；DST 沿用既有规则。
3. 「不再提醒」单一开关、不按档位区分。详情：有 due 时显示一个勾选框；清 due 一并清开关；说明文案整段删除。通知：所有提醒通知带同一个「不再提醒」按钮（Tier 1 插件按钮），点击=同一开关。持久化 `reminder: false`（缺省=开；恢复=删键）；任务级，改 due 不自动恢复；旧 `reminder_offset_days` 视同「开」、下次保存清理。
4. 文案：标题=任务名；「还有 7 天到期」/「还有 1 天到期」/「还有 1 小时到期」/「已到截止时间」。
5. 错过与首帧：运行中错过 → 每任务合并一条（due 已过→「已到截止时间」，否则按真实剩余时长），已过档记账；首帧（新建/改 due/升级/提醒开关重新开启）已过的提前档静默记账、不发送；首帧已过的「到期档」保留现状补一条（新建/升级等首次观察场景）。完成→重开（观察到非 open→open）不属首帧：沿用 1.5.0 语义，全部已过档静默记账、不补发，仅保留未来档。
6. 技术档位：Tier 1 插件按钮（点击仅通知在屏时有效，≈5 秒；通知中心/应用退出后点击无响应——已接受）；点通知本体=唤醒并前置窗口；错过窗口可用详情勾选框兜底。按钮链路最终在打包版实测。
7. 不做：snooze、全局开关、设置页提醒项、通知撤回、列表徽标。提醒仍仅应用运行时排程；fail-closed、ledger、完成取消等既有机制沿用。

### 冻结契约（实现依据）
- YAML：`reminder: false`=关（唯一写入值）；缺省/`true`=开；旧 `reminder_offset_days` 不再写入、任务任意元数据保存时移除。
- IPC：TaskView `reminder: null | false`（替换 `reminderOffsetDays`）；`update_task_metadata` 补丁 `reminder?: false | null`（false=写键、null=删键、省略=不变、true=非法）。
- 校验：无 due 时禁止 `reminder: false`；清 due 同时清 `reminder` 键。
- 事件类型（ledger 串）：`advance7`/`advance1`/`advance1h`/`due`；旧 `advance` 仅兼容读取、不再生成；事件键仍为 {task_id, canonical_due, event_type}，不得加字段。
- ledger：新增 `observed: {task_id → canonical_due}`（仅 open+开启+due 合法任务，仅完整快照更新/清理）；旧文件无该字段须可读。
- 通知：action type id `due-reminder`、action `remind-off` 标题「不再提醒」；`extra` 携带 `taskId`/`listFile`；标题=任务名。
- 开关回写后前端同步事件：`reminder-disabled`，payload `{ listFile, taskId }`；同时触发提醒运行时快照刷新。
- 点通知本体：显示并聚焦主窗口。

## 完成判据
- 四档事件生成与时间口径、DST、ledger 兼容与 `observed` 生命周期、首帧静默、错过合并与文案、`reminder:false` 契约与旧字段清理、动作注册与回写（按钮→关提醒+刷新；本体→前置窗口）均有实现与测试。
- 前端：勾选框语义/补丁、说明文案删除、`reminder-disabled` 刷新；`pnpm build` 与既有断言脚本通过。
- `cargo test --all-targets` 全绿；`git diff --check` 通过。
- PDR（01-spec / 02-interfaces / 04-current）同步；版本四处 bump 至 1.6.0。
- 用户验收（dev；按钮链路建议打包版）通过后归档。

## 步骤
- [x] 规划（grilling 定案；可行性核证：tauri-plugin-notification 2.6.0 桌面按钮可用，限制=仅通知在屏时点击可收取）
- [x] 骨架（契约冻结 + 端到端最小切片：勾选框 ↔ YAML ↔ 提醒暂停）
- [x] 细节（四档事件与文案、首帧/补发、通知按钮与回写、前端接线）
- [x] 异常（边界与失败路径、测试、oracle 审阅）
- [x] 回顾（PDR 同步、版本 bump、验收、归档）

## 结果
- 四档默认提醒（T-7d / T-1d / T-1h + 到期）与「不再提醒」单一开关（详情勾选框 + 通知按钮）落地；首帧静默与 done→open 抑制沿用；通知动作回写 + 前端 `reminder-disabled` 刷新；旧 `reminder_offset_days` 清理。
- oracle 审阅修正：并发写丢失更新（16 个存储写操作持进程级写锁）、已过事件立即复查（`next_poll_delay`）；补 DST / 关闭提醒测试。
- 验证：`cargo test --all-targets` **187/187**、`pnpm build`、断言全绿（taskMove 14 / completedGroups 16 / AppBar 15 / Agent 13）、`git diff --check` 通过。
- 用户验收通过（2026-10-13）；版本本地 bump **1.6.0**（提交 / 打 tag / 推送待用户流程）。

## Log
- 2026-10-13 grilling 定案（7 条共识 + 冻结契约）；按钮可行性来自插件 2.6.0 源码核证（PR #3671；仅通知在屏时点击可收取）。
- 2026-10-13 issue 创建；后端 / 前端两路并行实现。
- 2026-10-13 实现落地（后端 + 前端）。核对代码发现并修正一处语义回归：done→open 被误当首帧、会对逾期任务补发「已到截止时间」；已恢复 1.5.0 抑制语义（claim 全部已过档、不通知）并回移旧测试。
- 2026-10-13 合并验证：`cargo test --all-targets` 184/184、`pnpm build`、断言 agentTask 13 / appBar 15 / completedGroups 16 / taskMove 14、`git diff --check` 全通过。
- 2026-10-13 oracle 独立审阅派发（全量 diff）。
- 2026-10-13 oracle 审阅完成（无阻塞问题）：修正并发写丢失更新（16 个存储写操作持进程级写锁）、已过事件立即复查（`next_poll_delay`）、`ON_TIME_DRIFT` 注释与 DST / 关闭提醒测试；`cargo test --all-targets` 187/187。
- 2026-10-13 PDR 同步（01-spec / 02-interfaces / 04-current）；版本四处 bump 1.6.0；等待用户验收（dev / 打包版）。
- 2026-10-13 用户验收通过；issue 归档（-done）。
