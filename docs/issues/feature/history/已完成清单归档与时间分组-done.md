# 已完成清单归档与时间分组

> Status: done（用户验收通过 2026-10-28，随 **1.10.0** 发布）｜ 责任: agent ｜ 创建: 2026-10-28 ｜ 关联: PDR `docs/product/10-app/01-spec.md` FR-APP-18 / FR-APP-36、`docs/product/10-app/02-interfaces.md` §2 / §5.1 / §5.4

## 背景与目标

「已完成」视图目前按来源清单分组、组内按 `completed` 倒序，无归档能力、无时间分组。随完成量增长，旧完成项稀释了近期内容。

本次两件事：

1. **归档**：已完成任务可归档（task 级时间戳），页面提供「显示归档」开关，关闭时不渲染归档项；任务右键可归档 / 取消归档，清单右键可一次归档该清单下全部未归档的已完成任务。
2. **时间分组**：已完成视图中每个清单分组内部再按完成时间拆为「今天 / 七天内 / 更早」三个标签小节，配合更大行距与说明性标签。

顺带：任务 / 清单右键菜单改为分区（分隔线），「删除」独立成区，降低误点。

## 已对齐范围（2026-10-28 grilling 定案）

**归档语义**

- 归档为 task 级字段 `archived`（本地时间字符串，与既有 `completed` 同风格）；清单本身不落归档字段，清单右键＝批量操作。
- 仅顶层 task 可归档；子任务不参与、不受父任务归档影响，数据原样保留。
- 已归档 task 的右键菜单只保留 **打开详情 / 取消归档 / 删除**，隐藏「完成 / 取消完成」与「移动到清单」，并禁止拖拽（避免「已归档但未完成」的怪状态）。
- 旧数据无 `archived` 字段即视为未归档，**无需迁移**。

**可见性**

- 「显示归档」开关只作用于已完成区域；默认关闭；状态持久化到 `settings.yaml`（不新建数据文件）。
- 关闭时侧边栏「已完成」总数与清单分组标题的「名称 · 数量」**只统计可见项**（计数跟随开关）。
- 开关打开时，归档项仍按其 `completed` 落在原时间组内的**末尾**（组内未归档在前、归档沉底，各自按 `completed` 降序），以弱化样式 + 「已归档」角标区分；不另设「已归档」分组。

**时间分组**

- 基准 = 完成时间 `completed`；边界：今天＝本地自然日 00:00 起；七天内＝昨天至第 7 天前（不含今天）；更早＝第 8 天前及更早；缺失 / 无法解析的时间戳 → 更早。
- 跨天只在进入已完成视图或数据刷新时懒重算，不加午夜定时器（跨午夜可短暂滞后）。
- 层级：清单分组（可折叠，沿用现样式）＞ 时间标签（**不可折叠**，12px 弱化色说明性标签，上方留较大间距）；清单折叠时三个时间组一起收起。
- 空时间组整体隐藏；某清单下无可见 task 时该清单分组整体不渲染。

**批量与撤销**

- 清单右键「归档该清单的已完成任务」：只处理**未归档**的已完成 task，不覆盖已有归档时间。
- 不做确认弹窗，改用 **5 秒撤销 toast**；**单槽位**（只保留最近一次批量操作），撤销为前端快照回写批量取消归档。

**UI**

- 「显示归档」开关位于「已完成」面板头部、`h2 已完成` 同行右侧，原生 checkbox + 文案（复用 `.reminder-toggle` 样式），不新增 Switch 组件。
- 右键菜单四区（明细见 SPEC FR-APP-36）：`打开详情 / 编辑` ｜ `完成|取消完成 / 移动到清单▸ / 归档|取消归档` ｜ `Agent▸` ｜ `删除(danger)`；清单右键 `重命名 ｜ 删除`。
- `ContextMenu` 增 `separator?: true` 极简扩展（现只有 `{key,label,danger,disabled,checked,children,onSelect}`），不改既有调用方语义。

## 完成判据

- 归档 / 取消归档可用：任务右键与清单右键两条路径均原子写盘，未知字段与其它任务不受影响；归档项重启后仍为归档态。
- 「显示归档」关：归档项不渲染、计数不含；开：归档项在其时间组末尾以弱化样式呈现。开关状态重启后保持。
- 三个时间标签边界正确（今天 / 昨天–7 天前 / 8 天前及更早；缺失时间戳归「更早」），空组与无可见任务的清单分组不渲染。
- 归档项右键只剩三项、不可拖拽；未归档项行为不回退（分组折叠、取消完成回原清单、移动清单等不变）。
- 批量归档可用且带 5 秒撤销；单槽位语义正确。
- 右键菜单分区渲染正确，删除独立成区，键盘 / Esc / 外部点击关闭行为不回退。
- 验证：Rust 单测（settings 与批量归档）、前端纯逻辑断言、`pnpm build`、`cargo test --all-targets`、`git diff --check`。

## 技术备注

- 后端：`Settings` 增 `completed_show_archived: bool`（YAML `completed.show_archived`，缺失＝false）+ `save_completed_show_archived` 命令；新增 `archive_tasks` / `unarchive_tasks`（按 `listFile` + `taskIds`，一次重读一次原子写，跳过不存在 / 非 done 的 id，不报错）；`TaskView` 增可选 `archived`。
- 前端：`completedGroups.ts` 扩展为「清单分组 × 时间分桶」纯函数（可断言）；`App.tsx:3021-3105` 渲染改造；`openTaskMenu` / `buildTaskMenuItems`（`App.tsx:973-1080`）与清单菜单（`App.tsx:2828-2842`）加归档项与分隔；`ContextMenu.tsx` 支持分隔线。
- 样式在 `src/App.css`（全局单文件），已完成区样式见 `:447-538`。
- 文档：SPEC `01-spec.md` 增 FR-APP-36；`02-interfaces.md` 增任务字段 `archived`、批量命令、§5.4 `completed.show_archived`；收尾更新 `docs/current/app.md`。

## 结果

- 「已完成」视图内每个来源清单组头右键「归档该清单的已完成任务」（0 项禁用，带 5 秒单槽位撤销 toast）；任务右键「归档 / 取消归档」，已归档项仅保留「打开详情 / 取消归档 / 删除」且无完成复选框。
- 面板头部「显示归档」开关（默认关、存 `settings.yaml` 的 `completed.show_archived`，乐观更新 + 串行落盘 + 失败回滚）；关闭时不渲染归档项，侧栏与组头计数同步只算可见项。
- 每个来源清单内按 `completed` 分「今天 / 七天内 / 更早」三档（档距 24px、档间标签 12px muted，同档任务间距沿用普通清单）。
- 层级：组头 1.06rem / 650 / 主文本色 + 数量 pill；组体 20px 缩进 + 发丝线；组间距 30px；窄屏组头 1rem、缩进 12px。
- 任务与清单右键菜单按区分隔，「删除」独立成区（`ContextMenu` 新增 `separator`）。
- 验证：Rust 261/261、`completedGroups` 26/26、`pnpm build`、`cargo clippy -D warnings`、`git diff --check` 全绿；用户 2026-10-28 验收通过（含一轮 3 条缺陷返工）。

## 步骤

- [x] 规划（grilling 对齐 19 项决策；SPEC 变更已落稿）
- [x] 骨架（`archived` 字段 + 批量命令 + 开关渲染最小切片）
- [x] 细节（时间分组渲染、弱化样式、角标、菜单分区、撤销 toast）
- [x] 异常（缺失 / 非法 `completed`、空组、归档项被取消完成、批量并发、写盘失败）
- [x] 回顾（Current / CONTEXT 同步、issue 归档）

## Log

- 2026-10-28 grilling 对齐（用户逐条确认 Q1–Q19）：归档挂 task 级 `archived`、清单右键＝批量；时间基准 `completed`、边界 今天 / 昨天–7 天前 / 8 天前及更早；开关只作用已完成区、默认关、存 settings.yaml；归档项留在时间组内弱化 + 沉底；右键项按状态切换、清单右键提供对称取消；批量只处理未归档项、5 秒单槽位撤销 toast；空组 / 空清单分组不渲染；已归档项仅留 打开详情 / 取消归档 / 删除 且禁拖拽；跨天懒重算；时间标签不可折叠；开关用原生 checkbox；右键菜单四区分隔、删除独立成区，清单右键同步分区。
- 2026-10-28 SPEC 变更落稿（FR-APP-36 + 02-interfaces `archived` / 批量命令 / §5.4），待用户确认后进入实现。
- 2026-10-28 用户确认开工，分三泳道：后端（`src-tauri`）、前端纯逻辑（`completedGroups.ts` / `settings.ts`）、UI（`App.tsx` / `App.css` / `ContextMenu.tsx`），后两条先后错开以避开 `App.tsx` 写冲突。
- 2026-10-28 前端纯逻辑完成（@fixer）：`timeBucketOf` / `visibleTasksForGroup` / `bucketCompletedGroups` / `totalVisibleCompleted` + `AppSettings.completedShowArchived`；断言 16 → 26 全绿，`pnpm build` 通过。已定边界：未来时间戳归「今天」；带时区后缀的时间戳视为不可解析归「更早」；`archived` 存在（含空串）即视为已归档。
- 2026-10-28 后端完成（@fixer）：`TaskView.archived`、`archive_tasks` / `unarchive_tasks`（一次重读 + 一次原子写，跳过未知 / 非 done / 已归档）、`Settings.completed_show_archived` + `save_completed_show_archived`（false 删键）、取消完成时清除 `archived`；Rust 252 → 261 全绿，`clippy -D warnings` 干净。旁注：空 `taskIds` 直接返回不写盘；前端 `settings.ts` 已按 `completed_show_archived` 取值，与后端序列化字段名一致。
- 2026-10-28 UI 完成（@designer）：三档时间小节（12px 弱化标签、档距 24px、行距 8→14px）、归档项弱化 + 「已归档」角标 + 隐藏完成复选框 + 补左缩进、面板头部「显示归档」原生 checkbox（乐观更新 + 串行落盘 + 失败回滚）、计数改可见口径、任务/清单右键四区分隔（`ContextMenu` 增 `separator`）、5 秒单槽位撤销 toast。
- 2026-10-28 合并验证通过：`pnpm build`、`node src/completedGroups.verify.cjs` 26/26、`cargo test --all-targets` 261/261、`cargo clippy --all-targets -D warnings`、`git diff --check`。收尾同步 `docs/current/app.md`（已完成视图能力与测试口径 252→261）与 `CONTEXT.md`（新增术语「归档」）。**待用户交互验证后归档本条目。**
- 2026-10-28 用户验收未通过（3 条）：① 批量归档挂错层级——应在「已完成」视图内的可折叠来源清单组头右键，而非侧栏常规清单；② 同时间档内 task 间距应回归普通清单值，只有档与档之间放大；③ 来源清单组头与任务缺乏层级差。修复（@designer）：侧栏菜单回到 `重命名 ｜ 删除`，新增独立的 `completedGroupMenu` 挂到组头 `onContextMenu`（仅一项「归档该清单的已完成任务」，0 项 disabled，ids 取组内未归档项）；删除 `.completed-bucket .task-list` 的 14px 覆盖；组头升为一级（1.06rem / 650 / 主文本色 / 数量 pill）、组体 20px 缩进 + 发丝线、组间距 26→30px，窄屏缩进 12px。验证：`pnpm build`、26/26、`git diff --check` 全绿。
- 2026-10-28 用户复验通过；版本 1.9.1 → **1.10.0**，构建 release 产物（NSIS + 裸 exe）并推送 master + tag `v1.10.0`；条目归档 `feature/history`。
