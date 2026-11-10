# 新增清单入口

> Status: doing ｜ 责任: agent ｜ 创建: 2026-11-10 ｜ 关联: SPEC 10-app 新增清单入口

## 背景与目标

现状：清单条（`.lists-block` 内横向 `.list-nav`）下方有一条常显的「新建清单…」输入框 + `+` 按钮，输入框永久占位。

目标：把 `+` 入口移到清单条右端（滚动区之外），输入框默认隐藏，点击后在清单条下方展开（视觉与现有新增行一致），Enter 提交生效，失焦隐藏。

## 完成判据

- `+` 按钮位于清单条右端，与 `<ul class="list-nav">` 同级、在横向滚动容器之外，横滚时按钮不移动。
- 按钮与最右固定项「已删除」之间有 8px 间距 + 1px 分隔线。
- 输入框默认不渲染；点击 `+` 后在清单条下方出现，自动聚焦，视觉沿用现有 `.add-row input`。
- Enter 提交：trim 非空 → 创建清单 → 自动选中新清单 → 清空文字并隐藏输入框，焦点回到按钮。
- 空名 / 纯空格 Enter：不发请求，隐藏输入框。
- Esc：隐藏并丢弃草稿，焦点回到按钮。
- 失焦：隐藏并保留草稿，下次点开恢复；提交成功才清空。
- 同名清单允许创建（后端 `unique_file_name` 去重），不新增校验。
- 失败：沿用现有 `actionError` 提示行，输入框保持打开、文字保留。
- 输入框展开期间清单行仍可点击 / 横滚。
- 窄屏（≤760px）布局一致，无额外降级分支。
- `pnpm build` 通过；`cargo test` 通过（后端未改，确认无回归）。

## 步骤

- [x] 规划（完成判据与方案已对齐）
- [x] 骨架（最小可运行切片 + 证据；纯修复可跳过并注明理由）
- [x] 细节（主路径实现与质量）
- [x] 异常（失败路径、边界与稳定性）
- [x] 回顾（收尾回顾：文档同步）

## 测试

- 执行：`pnpm build`（tsc 类型检查 + vite 构建）→ 通过 ｜ `cargo test` → 286 passed, 0 failed
- 临时：代码路径逐条核对（点开 / 空名收起 / 同名 / Esc 丢弃草稿 / 失焦保留草稿 / 提交后自动选中）→ 通过；**未做运行时手工点击验证**，需在本机 app 内确认视觉与交互
- 落盘：无（项目无前端测试脚手架，Rust 侧未改动；引入 vitest 属独立 chore）

## 结果（完成后填写，可选）

- `src/App.tsx`：新增 `showNewListInput` / `newListButtonRef` / `toggleNewListInput`；`submitNewList` 改为空名收起不发请求、成功后用 `create_list` 返回文件名 `selectList` 自动选中；清单条改为 `.list-strip`（SortableList + 分隔线 + `+`），输入行条件渲染且 `autoFocus`。
- `src/App.css`：新增 `.list-strip` / `.list-strip > .list-nav` / `.list-strip-divider` / `.list-strip-add`，沿用 tokens，无新增动画。
- 文档：SPEC 新增 FR-APP-38；Current `app.md` 更新清单条描述并移除「新建清单后不自动选中」已知项；PRD 无变化。
- 验证缺陷：临时未做真实 UI 点击（无自动化脚手架）。

## Log

- 2026-11-10 建立条目；需求经 grilling 对齐（按钮右端、输入框独占一行、自动选中、草稿保留）。
- 2026-11-10 修正：失败提示复用 `actionError` 行，项目无 toast 通道。
- 2026-11-10 实现完成（`src/App.tsx` + `src/App.css`）；`pnpm build` / `cargo test`(286) 通过；SPEC FR-APP-38 与 Current `app.md` 已同步。
- 2026-11-10 用户验收通过；追加视觉修正：「+」按钮改为 40×40 正方形（共用 `.add-btn`，与「新增 todo」一致）。归档。
