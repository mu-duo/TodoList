# 优化跨清单拖拽浮层为图标

> Status: done（2026-10-21 用户验收通过） ｜ 责任: agent ｜ 创建: 2026-10-21 ｜ 关联: FR-APP-04

拖 task 到其他清单时，`DragOverlay` 继承源任务行整行宽度（默认窗口约 700×44），白色浮层 + 大阴影 + opacity .95，整块盖住右侧 340px 清单栏，导致看不清目标清单名；现有 `is-drop-target` 高亮被压住。目标：拖 task **离开源任务滚动区**后把浮层收缩为 32px 首字图标，露出清单名与高亮；源清单内排序保持现状。

2026-10-21 用户验收追加：触发从「进入非源清单条」改为「离开源任务滚动区 `.task-scroll`」（约 4px 容差）；「已完成」固定项成为任务拖拽落点，松手等同勾选完成。

## 完成判据

- 拖 task 离开源任务滚动区 `.task-scroll`（约 4px 容差）即收缩为 32px 圆角方块（标题首字，accent-soft 底 / accent-strong 字）；在源清单内保持整行卡片。
- 图标在清单条 / 条间 gap / 空白 / 「已完成」上保持，不闪回大卡；清单条间 gap 内无高亮、释放不移动任务。
- 拖到「已完成」固定项：高亮；松手等同勾选完成（`set_task_status` done），不移动文件、不触发同清单排序。
- 回到源任务滚动区 / 取消时反向展开为完整卡片；成功落入清单或「已完成」时图标 100ms 淡出、不飞回。
- 源清单内排序、鼠标 / 触屏 / 键盘现有行为、暗色主题、窄屏横向清单、reduced-motion 均无回归。
- `pnpm build` 通过；`node src/taskMove.verify.cjs` 全过（含 `isPointInRect` 断言）；CDP 三态截图 + 手工录屏证据留档。
- 收尾更新 `docs/current/app.md` 第 13 行（拖拽图标反馈 + 「已完成」落点）。

## 追加（2026-10-21）

- [x] 收缩触发改为「离开源任务滚动区 `.task-scroll`」，源清单内保持整行卡片；移除原 list-nav gap 视觉区逻辑。
- [x] 「已完成」固定项成为任务拖拽落点：高亮；松手等同勾选完成；不移动文件、不触发同清单排序。
- [x] `isPointInListZone` 替换为 `isPointInRect`（源区 4px 容差 + 已完成命中），并更新断言。

## 步骤

- [x] 规划（完成判据与方案已对齐）
- [x] 骨架（visual zone 纯函数 + 浮层收缩骨架 + 验证脚本）
- [x] 细节（抓取点非等比收缩 / 图标交叉淡入 / 100ms 原地淡出 / 主题与窄屏）
- [x] 异常（gap 不闪回、源清单洞、离开反向展开、取消不飞回、reduced-motion）
- [x] 回顾（收尾回顾：文档同步；用户实机验收通过）

## Log

- 2026-10-21 与用户完成 grilling，确认：32px 首字图标；非等比收缩 + 交叉淡入；仅非源清单触发；视觉区覆盖条间 gap；成功落下不飞回；PRD/SPEC 不动。
- 2026-10-21 实现：
  - `src/taskMove.ts` 新增 `isPointInListZone`（非源条并集外接框、源清单为洞、空集 / null → false），并抽出 `inRect` 复用命中测试。
  - `src/taskMove.verify.cjs` 新增 11 条视觉区断言（垂直 / 横向 gap、边界、空集、null、源清单洞、源清单居中洞外 gap）；共 25 条全过。
  - `src/components/SortableList.tsx`：`onDragStart` 传出事件；新增 `overlayCollapsed` / `overlayStyle`；`DragOverlay` 传 `pointerEvents:none`；收缩态落下使用 100ms **零位移**淡出关键帧（dnd-kit 会在 dragEnd 同步卸载浮层，`dropAnimation=null` 无法保留淡出时间；该关键帧不做位移，等价于不飞回）。
  - `src/App.tsx`：新增 `dragOverListZone` state/ref、`dragMetrics`；在拖拽起点直接量源任务行（`activatorEvent.target.closest('.task-card')`）取实际宽高，并配合激活事件算抓取点（dnd-kit 的 `active.rect.current.initial` 在 onDragStart 同步阶段尚未写入）；拖拽 move 同一份 `listNavRects()` 算落点与视觉区；结束清理状态；浮层内容加首字图标（空标题「·」、完成态 .55、`aria-hidden`）。
  - `src/App.css`：卡片以抓取点为 origin、140ms 展开 / 160ms 收缩；图标 32×32 / radius 10 / accent token / 小阴影；收缩态移除大 filter 阴影。
  - `docs/current/app.md` 第 13 行补充图标浮层事实。
- 证据：`node src/taskMove.verify.cjs` → `25 assertions passed`；`pnpm build` → `tsc && vite build` 成功（2.44s）。
- 未执行：Tauri / dev server 实机与 CDP 截图；缝隙释放与 100ms 淡出的实际观感待手工复验。
- 已知降级：缝隙释放 / 取消时，dnd-kit 会同步卸载浮层，无法稳定「先展开再回位」，退化为图标原地 100ms 淡出（零位移、绝不飞回）；成功落入同样走这条零位移淡出。
- 2026-10-21 复核修正：
  - `collapsedDropRef` 改为随 `overlayCollapsed` 回落为 false（原先只置 true）：避免先经过清单、再拖回空白 / 源清单落下时仍错误走收缩淡出；只有落下瞬间处于收缩态才不飞回。
  - 浮层 `pointer-events:none` 后补 `body.is-dragging` 全局 grabbing 光标，避免拖拽光标回落。
  - 复跑 `pnpm build` 通过、`node src/taskMove.verify.cjs` 25 断言全过、`git diff --check` 仅 CRLF 提示。
- 2026-10-21 用户验收追加范围：
  - 收缩触发改为「离开源任务滚动区 `.task-scroll`」即生效（4px 容差），源清单内保持整行卡片；原 list-nav gap 视觉区方案被取代。
  - 「已完成」固定项成为任务拖拽落点：高亮；松手等同勾选完成（`set_task_status` done），不移动文件、不触发同清单排序。
  - 实施计划：`isPointInListZone` → `isPointInRect`；新增 `taskScrollRef` / 已完成 footer ref / `dragOutsideSource` / `dragOverCompleted`；更新 Current 与断言。
- 2026-10-21 追加范围实现：
  - `src/taskMove.ts`：删除 `isPointInListZone`，新增 `isPointInRect(pointer, rect, margin = 0)`（端点含边界、margin 向外扩张、null → false）；`hitTestListFile` 复用它。
  - `src/taskMove.verify.cjs`：删除 11 条 list-zone 断言；新增内部 / 四角边界 / 外侧 1px / 4px 容差含边界 / 容差外 / 负容差 / null 共 7 条；总计 21 条全过（原 14 条保留）。
  - `src/App.tsx`：`dragOverListZone` → `dragOutsideSource`（普通视图 `.task-scroll` ref + 4px 容差取反）；新增 `completedFooterRef` / `dragOverCompleted` / `applyDragOverCompleted`；drag start/end 重置；结束优先处理「已完成」并置 `dragMoveHandledRef` 后 return；footer 加 ref、`is-drop-target` 类与 click 抑制。
  - `src/App.css`：补 `.list-nav-item.is-fixed.is-drop-target .list-nav-select` 的 accent 文字色。
  - `docs/current/app.md` 第 13 行改为「离开源任务列表区收缩 + 拖到已完成＝勾选完成」。
- 证据：`node src/taskMove.verify.cjs` → `21 assertions passed`；`pnpm build` → `tsc && vite build` 成功（238ms）。
- 未执行：Tauri / dev server 实机与 CDP 截图；源区 4px 容差手感、「已完成」落点高亮 / 不切视图、键盘拖拽不收缩待手工复验。
- 2026-10-21 用户实机验收通过（源区内外收缩、清单 / gap / 空白保持、已完成落点、跨清单移动、源内排序）。版本同步 **1.8.2**；`./scripts/build-release.ps1 -Version 1.8.2` 构建成功，产物 `release/TodoList_1.8.2_x64-setup.exe` / `TodoList_1.8.2_x64.exe` + SHA256；未做 CDP 录屏留档，以用户验收为准。归档 `-done`。

