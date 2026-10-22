# 详情面板：子板块统一结构与折叠

> Status: done ｜ 责任: agent ｜ 创建: 2026-10-22 ｜ 关联: 右侧边栏-清单导航与任务详情、已完成清单分组折叠、ADR-0006

## 背景与目标

任务详情里的各子板块结构各异（`label.field` / `section.metadata-editor` / `div.subtask-block` / `dl.detail-meta` / `section.agent-section`），视觉与 DOM 不统一，且「任务属性」占用大量纵向空间。目标：所有子板块统一为 `title + div.content` 两段式，并全部支持折叠——「任务属性」默认折叠，其余默认展开；折叠状态持久化到 `settings.yaml`。

## 板块与 key

| key | 标题 | content 形态 | 默认 |
| --- | --- | --- | --- |
| `title` | 标题 | 无边框（`input.edit-title` 顶格） | 展开 |
| `metadata` | 任务属性（含 `随改随存` 说明） | 边框包裹（多字段 grid） | **折叠** |
| `subtasks` | 子任务 · N | 边框包裹（列表 + 新增输入行） | 展开 |
| `notes` | 备注 | 无边框（`textarea.edit-notes` 顶格） | 展开 |
| `meta` | 其他信息 | 无边框（状态 / 完成时间 / ID 的 `dl`） | 展开 |
| `agent` | Agent 协助 | 边框包裹（`AgentTaskPanel`） | 展开 |

## 完成判据

- 每块 = `h3 > button[aria-expanded]`（含 chevron，收起时旋转）+ `div.content`；收起时不渲染 content；
- 简单板块（标题 / 备注）content 内不再重复字段标签，控件可访问名由板块标题承担（`aria-labelledby`）；复杂板块内子字段（优先级 / 截止日期 / 标签）保留各自标签；
- 折叠状态**全局共用一份**，持久化 `settings.yaml` 的 `detail.collapsed`；首次无该 key 时 = `["metadata"]`；
- 与 `completed.collapsed` 的差异：**空数组必须写盘**（`collapsed: []`），不清除 key——因为此处默认值不是“全展开”，缺 key 与“全部展开”语义不同；
- 写入失败回滚到上一次成功状态并提示（沿用 `completedCollapsedError` 模式）；
- 既有详情功能不回退：即时保存、Enter/Esc、子任务增删改勾、截止/标签/优先级编辑、Agent 面板；
- `pnpm build` 与 `cargo check` 通过；接口文档 `docs/product/10-app/02-interfaces.md` 增补该 key 与命令。

## 步骤

- [x] 规划（板块清单 / 折叠范围 / 默认值 / 持久化粒度 / 边框规则已与用户逐条对齐）
- [x] 骨架（Rust settings key + 命令；前端 `DetailSection` 组件与六块落位）
- [x] 细节（样式统一、边框规则、a11y、持久化链路）
- [x] 异常（写盘失败回滚、未知 key 过滤、无选中任务空态）
- [x] 回顾（文档同步、条目归档）

## Log

- 2026-10-22 用户提出：任务属性新增折叠（默认折叠），并统一详情内子板块为 title + content 形态。对齐结论：六块（标题/任务属性/子任务/备注/其他信息/Agent 协助）全部统一且全部可折叠；简单板块无边框，复杂板块 content 加边框包裹；折叠状态持久化 settings.yaml 且全局共用；折叠行沿用已完成分组形态。
- 2026-10-22 实施：新增 `src/detailSections.ts`（key 集合 / 默认值 / 清洗 / 切换计算）与 `src/components/DetailSection.tsx`；`renderDetail()` 重构为六块；折叠 wiring 照「已完成分组」写法（乐观更新 + 串行保存 + 失败回滚）；`src/settings.ts` 增 `detailCollapsed`。Rust 侧 `Settings.detail_collapsed` + `parse_detail_collapsed` + `save_detail_collapsed_in` + 命令 `save_detail_collapsed`，语义差异（空数组写盘、不清 key）已实现并有单测。接口文档新增 §5.2（托盘节顺延 §5.3 并同步交叉引用）、Current 与 CSS 头部契约同步。
- 2026-10-22 收口：补齐「Agent 协助」收起时点任务行尾机器人图标无滚动目标的问题（定位前先展开该板块，双 rAF 后再 scrollIntoView）。证据：`pnpm build` 通过（tsc + vite，53 modules）、`cargo test --lib` 240 passed / 0 failed。
- 2026-10-22 用户验收后微调边框规则：子任务不再整体 boxed，改为**每条子任务各自带边框**（列表 gap 6px；编辑态让位给输入框边框避免双层）；「其他信息」改为 boxed。boxed 集合现为 任务属性 / 其他信息 / Agent 协助。
- 2026-10-22 用户验收通过，条目归档。
